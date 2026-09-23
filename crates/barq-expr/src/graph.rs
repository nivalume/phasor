//! A factor set compiled once into a deduplicated dependency graph.
//!
//! [`crate::FactorSet::compile`] turns every expression tree into nodes of one
//! shared graph:
//!
//! * **Hash-consed.** A node is identified by its operator and its operand
//!   nodes, so `Ref($close, 1)` appearing in forty Alpha158 factors, or `a+b`
//!   next to `Add(a, b)`, is one node computed once.
//! * **Validated.** Window lengths, offsets and other literal parameters are
//!   checked here, so a bad factor fails at compile time rather than after the
//!   rest of the set has been evaluated.
//! * **Constant-folded.** Elementwise operators whose operands are all
//!   literals become literals.
//! * **Scheduled.** Nodes are stored in dependency order, and every computed
//!   node gets a scratch slot that is reused once the node's last consumer has
//!   run. A symbol's whole factor set therefore needs only [`Graph::slots`]
//!   series of scratch, however many factors and intermediates it has.
//!
//! Every operator is currently a per-symbol time-series operator, so the graph
//! is evaluated one symbol (column) at a time, symbols in parallel. A future
//! cross-sectional operator (one output row reads every symbol) would have to
//! split the schedule into stages at that node: columns in parallel within a
//! stage, one cross-sectional pass between stages.

use std::collections::HashMap;

use barq_core::error::{Error, Result};

use crate::parse::Expression;

/// Index of a node in [`Graph::nodes`].
pub(crate) type NodeId = usize;

/// A window argument; `0` in the source means the whole series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Window {
    Fixed(usize),
    All,
}

impl Window {
    pub(crate) fn resolve(self, n_rows: usize) -> usize {
        match self {
            Self::Fixed(window) => window,
            Self::All => n_rows.max(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UnaryOp {
    Neg,
    Abs,
    Sign,
    Log,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    And,
    Or,
    Greater,
    Less,
}

/// Per-symbol rolling reductions over a trailing window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Rolling {
    Mean,
    Std,
    Var,
    Sum,
    Max,
    Min,
    /// The quantile's bits, so the operator stays hashable.
    Quantile(u64),
    Med,
    Mad,
    Count,
    Skew,
    Kurt,
    Wma,
    Rank,
    IdxMax,
    IdxMin,
    Slope,
    Rsquare,
    Resi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Op {
    /// Index into [`Graph::fields`].
    Field(usize),
    /// A literal's bits.
    Const(u64),
    Unary(UnaryOp),
    Binary(BinaryOp),
    If,
    Ref(i64),
    Delta(usize),
    /// `None` is the expanding EMA (`EMA(x, 0)`), otherwise the alpha's bits.
    Ema(Option<u64>),
    Rolling(Rolling, Window),
    Pair {
        corr: bool,
        window: Window,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(crate) op: Op,
    pub(crate) inputs: Vec<NodeId>,
    /// Scratch slot for a computed node; `None` for fields and literals.
    pub(crate) slot: Option<usize>,
    /// Output positions that take this node's value.
    pub(crate) outputs: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct Graph {
    /// In dependency order: every node's inputs come before it.
    pub(crate) nodes: Vec<Node>,
    /// The node each factor reads, in factor order.
    pub(crate) outputs: Vec<NodeId>,
    /// Input field names, indexed by [`Op::Field`].
    pub(crate) fields: Vec<String>,
    /// Scratch series needed per symbol.
    pub(crate) slots: usize,
}

impl Graph {
    /// Compiles named expressions (already parsed and arity-checked) in
    /// output order.
    pub(crate) fn compile<'e>(
        factors: impl IntoIterator<Item = (&'e str, &'e Expression)>,
    ) -> Result<Self> {
        let mut builder = Builder::default();
        let mut outputs = Vec::new();
        for (name, expr) in factors {
            let node = builder.node(expr).map_err(|error| match error {
                Error::Invariant(message) => {
                    Error::Invariant(format!("factor {name:?}: {message}"))
                }
                other => other,
            })?;
            outputs.push(node);
        }
        let mut graph = Self {
            nodes: builder.nodes,
            outputs,
            fields: builder.fields,
            slots: 0,
        };
        graph.schedule();
        Ok(graph)
    }

    /// Assigns each computed node a scratch slot, reusing a slot once the
    /// node that held it has had its last reader.
    fn schedule(&mut self) {
        let mut last_read = vec![None; self.nodes.len()];
        for (at, node) in self.nodes.iter().enumerate() {
            for &input in &node.inputs {
                last_read[input] = Some(at);
            }
        }
        for (position, &node) in self.outputs.iter().enumerate() {
            self.nodes[node].outputs.push(position);
        }
        let mut free = Vec::new();
        let mut slots = 0;
        for at in 0..self.nodes.len() {
            if matches!(self.nodes[at].op, Op::Field(_) | Op::Const(_)) {
                continue;
            }
            // Allocate before releasing this node's inputs, so a node never
            // writes into a slot it is reading.
            let slot = free.pop().unwrap_or_else(|| {
                slots += 1;
                slots - 1
            });
            self.nodes[at].slot = Some(slot);
            let mut released = Vec::new();
            for &input in &self.nodes[at].inputs {
                if last_read[input] == Some(at) && !released.contains(&input) {
                    released.push(input);
                    if let Some(slot) = self.nodes[input].slot {
                        free.push(slot);
                    }
                }
            }
            // A node nothing reads is only an output, written as it is computed.
            if last_read[at].is_none() {
                free.push(slot);
            }
        }
        self.slots = slots;
    }
}

#[derive(Default)]
struct Builder {
    nodes: Vec<Node>,
    interned: HashMap<(Op, Vec<NodeId>), NodeId>,
    fields: Vec<String>,
}

impl Builder {
    fn intern(&mut self, op: Op, inputs: Vec<NodeId>) -> NodeId {
        if let Some(folded) = self.fold(op, &inputs) {
            return self.intern(Op::Const(folded.to_bits()), Vec::new());
        }
        let key = (op, inputs);
        if let Some(&id) = self.interned.get(&key) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            op,
            inputs: key.1.clone(),
            slot: None,
            outputs: Vec::new(),
        });
        self.interned.insert(key, id);
        id
    }

    /// The value of an elementwise operator over literal operands.
    fn fold(&self, op: Op, inputs: &[NodeId]) -> Option<f64> {
        let literal = |id: NodeId| match self.nodes[id].op {
            Op::Const(bits) => Some(f64::from_bits(bits)),
            _ => None,
        };
        let values: Option<Vec<f64>> = inputs.iter().map(|&id| literal(id)).collect();
        let values = values?;
        match op {
            Op::Unary(op) => Some(unary(op, values[0])),
            Op::Binary(op) => Some(binary(op, values[0], values[1])),
            Op::If => Some(if_else(values[0], values[1], values[2])),
            _ => None,
        }
    }

    fn node(&mut self, expr: &Expression) -> Result<NodeId> {
        Ok(match expr {
            Expression::Number(bits) => self.intern(Op::Const(*bits), Vec::new()),
            Expression::Field(name) => {
                let index = match self.fields.iter().position(|field| field == name) {
                    Some(index) => index,
                    None => {
                        self.fields.push(name.clone());
                        self.fields.len() - 1
                    }
                };
                self.intern(Op::Field(index), Vec::new())
            }
            Expression::Unary('+', child) => self.node(child)?,
            Expression::Unary('-', child) => {
                let child = self.node(child)?;
                self.intern(Op::Unary(UnaryOp::Neg), vec![child])
            }
            Expression::Unary(op, _) => {
                return Err(Error::Invariant(format!("unsupported unary {op}")));
            }
            Expression::Binary(op, left, right) => {
                let op = match op.as_str() {
                    "+" => BinaryOp::Add,
                    "-" => BinaryOp::Sub,
                    "*" => BinaryOp::Mul,
                    "/" => BinaryOp::Div,
                    "**" => BinaryOp::Pow,
                    ">" => BinaryOp::Gt,
                    "<" => BinaryOp::Lt,
                    ">=" => BinaryOp::Ge,
                    "<=" => BinaryOp::Le,
                    "==" => BinaryOp::Eq,
                    "!=" => BinaryOp::Ne,
                    "&" => BinaryOp::And,
                    "|" => BinaryOp::Or,
                    other => {
                        return Err(Error::Invariant(format!("unsupported operator {other}")));
                    }
                };
                let inputs = vec![self.node(left)?, self.node(right)?];
                self.intern(Op::Binary(op), inputs)
            }
            Expression::Call(name, args) => self.call(name, args)?,
        })
    }

    fn call(&mut self, name: &str, args: &[Expression]) -> Result<NodeId> {
        let unary = match name {
            "Abs" => Some(UnaryOp::Abs),
            "Sign" => Some(UnaryOp::Sign),
            "Log" => Some(UnaryOp::Log),
            "Not" => Some(UnaryOp::Not),
            _ => None,
        };
        if let Some(op) = unary {
            let input = self.node(&args[0])?;
            return Ok(self.intern(Op::Unary(op), vec![input]));
        }
        let binary = match name {
            "Power" => Some(BinaryOp::Pow),
            "Add" => Some(BinaryOp::Add),
            "Sub" => Some(BinaryOp::Sub),
            "Mul" => Some(BinaryOp::Mul),
            "Div" => Some(BinaryOp::Div),
            "Gt" => Some(BinaryOp::Gt),
            "Ge" => Some(BinaryOp::Ge),
            "Lt" => Some(BinaryOp::Lt),
            "Le" => Some(BinaryOp::Le),
            "Eq" => Some(BinaryOp::Eq),
            "Ne" => Some(BinaryOp::Ne),
            "And" => Some(BinaryOp::And),
            "Or" => Some(BinaryOp::Or),
            "Greater" => Some(BinaryOp::Greater),
            "Less" => Some(BinaryOp::Less),
            _ => None,
        };
        if let Some(op) = binary {
            let inputs = vec![self.node(&args[0])?, self.node(&args[1])?];
            return Ok(self.intern(Op::Binary(op), inputs));
        }
        let rolling = match name {
            "Mean" => Some(Rolling::Mean),
            "Std" => Some(Rolling::Std),
            "Var" => Some(Rolling::Var),
            "Sum" => Some(Rolling::Sum),
            "Max" => Some(Rolling::Max),
            "Min" => Some(Rolling::Min),
            "Quantile" => {
                let q = number(&args[2], "quantile")?;
                if !(0.0..=1.0).contains(&q) {
                    return Err(Error::Invariant("quantile must be in 0..=1".into()));
                }
                Some(Rolling::Quantile(q.to_bits()))
            }
            "Med" => Some(Rolling::Med),
            "Mad" => Some(Rolling::Mad),
            "Count" => Some(Rolling::Count),
            "Skew" => Some(Rolling::Skew),
            "Kurt" => Some(Rolling::Kurt),
            "WMA" => Some(Rolling::Wma),
            "Rank" => Some(Rolling::Rank),
            "IdxMax" => Some(Rolling::IdxMax),
            "IdxMin" => Some(Rolling::IdxMin),
            "Slope" => Some(Rolling::Slope),
            "Rsquare" => Some(Rolling::Rsquare),
            "Resi" => Some(Rolling::Resi),
            _ => None,
        };
        if let Some(kind) = rolling {
            let window = window(&args[1])?;
            let input = self.node(&args[0])?;
            return Ok(self.intern(Op::Rolling(kind, window), vec![input]));
        }
        Ok(match name {
            "If" => {
                let inputs = vec![
                    self.node(&args[0])?,
                    self.node(&args[1])?,
                    self.node(&args[2])?,
                ];
                self.intern(Op::If, inputs)
            }
            "Ref" => {
                let offset = integer(&args[1], "Ref offset")?;
                let input = self.node(&args[0])?;
                self.intern(Op::Ref(offset), vec![input])
            }
            "Delta" => {
                let offset = integer(&args[1], "Delta offset")?;
                let offset = usize::try_from(offset)
                    .map_err(|_| Error::Invariant("Delta offset cannot be negative".into()))?;
                let input = self.node(&args[0])?;
                self.intern(Op::Delta(offset), vec![input])
            }
            "EMA" => {
                let parameter = number(&args[1], "EMA parameter")?;
                if parameter < 0.0 {
                    return Err(Error::Invariant("EMA parameter cannot be negative".into()));
                }
                let alpha = (parameter > 0.0).then(|| {
                    if parameter < 1.0 {
                        parameter
                    } else {
                        2.0 / (parameter + 1.0)
                    }
                });
                let input = self.node(&args[0])?;
                self.intern(Op::Ema(alpha.map(f64::to_bits)), vec![input])
            }
            "Corr" | "Cov" => {
                let window = window(&args[2])?;
                let inputs = vec![self.node(&args[0])?, self.node(&args[1])?];
                self.intern(
                    Op::Pair {
                        corr: name == "Corr",
                        window,
                    },
                    inputs,
                )
            }
            _ => return Err(Error::Invariant(format!("unsupported operator {name}"))),
        })
    }
}

fn number(expr: &Expression, label: &str) -> Result<f64> {
    match expr {
        Expression::Number(bits) => Ok(f64::from_bits(*bits)),
        Expression::Unary('-', inner) => Ok(-number(inner, label)?),
        _ => Err(Error::Invariant(format!(
            "{label} must be a numeric literal"
        ))),
    }
}

fn integer(expr: &Expression, label: &str) -> Result<i64> {
    let value = number(expr, label)?;
    if value.fract() != 0.0 || !value.is_finite() {
        return Err(Error::Invariant(format!("{label} must be an integer")));
    }
    Ok(value as i64)
}

fn window(expr: &Expression) -> Result<Window> {
    let value = integer(expr, "window")?;
    match usize::try_from(value) {
        Ok(0) => Ok(Window::All),
        Ok(window) => Ok(Window::Fixed(window)),
        Err(_) => Err(Error::Invariant("window cannot be negative".into())),
    }
}

#[allow(
    clippy::float_cmp,
    reason = "Qlib's Not is an exact comparison with zero"
)]
pub(crate) fn unary(op: UnaryOp, value: f64) -> f64 {
    match op {
        UnaryOp::Neg => -value,
        UnaryOp::Abs => value.abs(),
        UnaryOp::Sign => {
            if value == 0.0 {
                0.0
            } else {
                value.signum()
            }
        }
        UnaryOp::Log => value.ln(),
        UnaryOp::Not => f64::from(value == 0.0),
    }
}

#[allow(
    clippy::float_cmp,
    reason = "Qlib's Eq/Ne and numeric boolean operators use exact NumPy comparisons"
)]
pub(crate) fn binary(op: BinaryOp, a: f64, b: f64) -> f64 {
    match op {
        BinaryOp::Add => a + b,
        BinaryOp::Sub => a - b,
        BinaryOp::Mul => a * b,
        BinaryOp::Div => a / b,
        BinaryOp::Pow => a.powf(b),
        BinaryOp::Gt => f64::from(a > b),
        BinaryOp::Lt => f64::from(a < b),
        BinaryOp::Ge => f64::from(a >= b),
        BinaryOp::Le => f64::from(a <= b),
        BinaryOp::Eq => f64::from(a == b),
        BinaryOp::Ne => f64::from(a != b),
        BinaryOp::And => f64::from(a != 0.0 && b != 0.0),
        BinaryOp::Or => f64::from(a != 0.0 || b != 0.0),
        BinaryOp::Greater => {
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a.max(b)
            }
        }
        BinaryOp::Less => {
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a.min(b)
            }
        }
    }
}

pub(crate) fn if_else(condition: f64, yes: f64, no: f64) -> f64 {
    if condition != 0.0 { yes } else { no }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn graph(sources: &[&str]) -> Result<Graph> {
        let parsed: Vec<Expression> = sources.iter().map(|s| parse(s).unwrap()).collect();
        Graph::compile(parsed.iter().map(|expr| ("f", expr)))
    }

    #[test]
    fn shared_subexpressions_and_spellings_are_one_node() {
        let g = graph(&[
            "Ref($close,1)/$close",
            "Div(Ref($close,1), $close)",
            "+Ref($close, 1)",
        ])
        .unwrap();
        // $close, Ref, Div.
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.outputs[0], g.outputs[1]);
        assert_eq!(g.nodes[g.outputs[1]].outputs, [0, 1]);
    }

    #[test]
    fn literals_fold_and_take_no_slot() {
        let g = graph(&["-(1 + 2) * 3", "If(1 > 2, 4, 5)"]).unwrap();
        let value = |id: NodeId| match g.nodes[id].op {
            Op::Const(bits) => f64::from_bits(bits),
            ref other => panic!("not folded: {other:?}"),
        };
        assert_eq!(value(g.outputs[0]).to_bits(), (-9.0_f64).to_bits());
        assert_eq!(value(g.outputs[1]).to_bits(), 5.0_f64.to_bits());
        assert_eq!(g.slots, 0);
    }

    #[test]
    fn slots_are_reused_after_the_last_read() {
        // A chain of ten operators never needs more than two live series.
        let g = graph(&["Abs(Abs(Abs(Abs(Abs(Abs(Abs(Abs(Abs(Abs($close))))))))))"]).unwrap();
        assert_eq!(g.slots, 2);
        for node in &g.nodes {
            if let (Some(slot), [input]) = (node.slot, node.inputs.as_slice()) {
                assert_ne!(Some(slot), g.nodes[*input].slot);
            }
        }
    }

    #[test]
    fn bad_parameters_fail_at_compile_time() {
        for (source, message) in [
            ("Mean($close, 2.5)", "integer"),
            ("Mean($close, -1)", "negative"),
            ("Quantile($close, 5, 1.5)", "0..=1"),
            ("EMA($close, -1)", "negative"),
            ("Delta($close, -1)", "negative"),
            ("Ref($close, $open)", "literal"),
        ] {
            let error = graph(&[source]).unwrap_err().to_string();
            assert!(error.contains(message), "{source}: {error}");
        }
    }
}
