use phasor_core::error::Result;

use crate::FactorSet;

/// Compiles Qlib's default Alpha158 feature library.
///
/// The formulas and output names follow `Alpha158DL.get_feature_config()`:
/// nine candlestick features, four current-price features, and 29 rolling
/// families over five windows.
///
/// # Errors
///
/// Propagates an internal expression parse error.
pub fn alpha158() -> Result<FactorSet> {
    let mut factors = vec![
        ("KMID".into(), "($close-$open)/$open".into()),
        ("KLEN".into(), "($high-$low)/$open".into()),
        ("KMID2".into(), "($close-$open)/($high-$low+1e-12)".into()),
        ("KUP".into(), "($high-Greater($open,$close))/$open".into()),
        (
            "KUP2".into(),
            "($high-Greater($open,$close))/($high-$low+1e-12)".into(),
        ),
        ("KLOW".into(), "(Less($open,$close)-$low)/$open".into()),
        (
            "KLOW2".into(),
            "(Less($open,$close)-$low)/($high-$low+1e-12)".into(),
        ),
        ("KSFT".into(), "(2*$close-$high-$low)/$open".into()),
        (
            "KSFT2".into(),
            "(2*$close-$high-$low)/($high-$low+1e-12)".into(),
        ),
    ];
    for field in ["open", "high", "low", "vwap"] {
        factors.push((
            format!("{}0", field.to_ascii_uppercase()),
            format!("${field}/$close"),
        ));
    }
    let mut rolling = Vec::new();
    for window in [5, 10, 20, 30, 60] {
        let d = window;
        for (name, expression) in [
            ("ROC", format!("Ref($close,{d})/$close")),
            ("MA", format!("Mean($close,{d})/$close")),
            ("STD", format!("Std($close,{d})/$close")),
            ("BETA", format!("Slope($close,{d})/$close")),
            ("RSQR", format!("Rsquare($close,{d})")),
            ("RESI", format!("Resi($close,{d})/$close")),
            ("MAX", format!("Max($high,{d})/$close")),
            ("MIN", format!("Min($low,{d})/$close")),
            ("QTLU", format!("Quantile($close,{d},0.8)/$close")),
            ("QTLD", format!("Quantile($close,{d},0.2)/$close")),
            ("RANK", format!("Rank($close,{d})")),
            (
                "RSV",
                format!("($close-Min($low,{d}))/(Max($high,{d})-Min($low,{d})+1e-12)"),
            ),
            ("IMAX", format!("IdxMax($high,{d})/{d}")),
            ("IMIN", format!("IdxMin($low,{d})/{d}")),
            ("IMXD", format!("(IdxMax($high,{d})-IdxMin($low,{d}))/{d}")),
            ("CORR", format!("Corr($close,Log($volume+1),{d})")),
            (
                "CORD",
                format!("Corr($close/Ref($close,1),Log($volume/Ref($volume,1)+1),{d})"),
            ),
            ("CNTP", format!("Mean($close>Ref($close,1),{d})")),
            ("CNTN", format!("Mean($close<Ref($close,1),{d})")),
            (
                "CNTD",
                format!("Mean($close>Ref($close,1),{d})-Mean($close<Ref($close,1),{d})"),
            ),
            (
                "SUMP",
                format!(
                    "Sum(Greater($close-Ref($close,1),0),{d})/(Sum(Abs($close-Ref($close,1)),{d})+1e-12)"
                ),
            ),
            (
                "SUMN",
                format!(
                    "Sum(Greater(Ref($close,1)-$close,0),{d})/(Sum(Abs($close-Ref($close,1)),{d})+1e-12)"
                ),
            ),
            (
                "SUMD",
                format!(
                    "(Sum(Greater($close-Ref($close,1),0),{d})-Sum(Greater(Ref($close,1)-$close,0),{d}))/(Sum(Abs($close-Ref($close,1)),{d})+1e-12)"
                ),
            ),
            ("VMA", format!("Mean($volume,{d})/($volume+1e-12)")),
            ("VSTD", format!("Std($volume,{d})/($volume+1e-12)")),
            (
                "WVMA",
                format!(
                    "Std(Abs($close/Ref($close,1)-1)*$volume,{d})/(Mean(Abs($close/Ref($close,1)-1)*$volume,{d})+1e-12)"
                ),
            ),
            (
                "VSUMP",
                format!(
                    "Sum(Greater($volume-Ref($volume,1),0),{d})/(Sum(Abs($volume-Ref($volume,1)),{d})+1e-12)"
                ),
            ),
            (
                "VSUMN",
                format!(
                    "Sum(Greater(Ref($volume,1)-$volume,0),{d})/(Sum(Abs($volume-Ref($volume,1)),{d})+1e-12)"
                ),
            ),
            (
                "VSUMD",
                format!(
                    "(Sum(Greater($volume-Ref($volume,1),0),{d})-Sum(Greater(Ref($volume,1)-$volume,0),{d}))/(Sum(Abs($volume-Ref($volume,1)),{d})+1e-12)"
                ),
            ),
        ] {
            rolling.push((format!("{name}{d}"), expression));
        }
    }
    // Qlib groups by operator first (ROC5..ROC60, then MA5..MA60), while the
    // construction above keeps each window's formulas together and readable.
    for operator in 0..29 {
        for window in 0..5 {
            factors.push(rolling[window * 29 + operator].clone());
        }
    }
    FactorSet::compile(factors)
}
