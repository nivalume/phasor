---
title: 过拟合检验：PSR、DSR、PBO
kind: algorithm
summary: 在每根 K 线的 Sharpe 上算 PSR、期望最大 Sharpe 与 DSR，并用 CSCV 枚举样本内外划分得到 PBO。
parent: phasor-analysis
related: [metrics, sweep-aggregation, research-gates, exercises]
tags: [statistics, overfitting, sharpe]
sources:
  - crates/phasor-analysis/src/robust.rs
order: 1104
---

# 过拟合检验：PSR、DSR、PBO

扫了一千组参数后，"最好"的那组有多少是运气？`robust.rs` 是一个独立模块（只依赖 `nanmean`/`nanstd`），给出三种回答：单个 Sharpe 的显著性（PSR）、按试验次数打折后的显著性（DSR）、样本内选赢家在样本外失败的概率（PBO）。

## 位置

- `crates/phasor-analysis/src/robust.rs`：`probabilistic_sharpe`、`expected_max_sharpe`、`deflated_sharpe`、`pbo_cscv`；`norm_cdf`、`norm_ppf`；`erfc` 内部 62–89

## 公式

这里的 Sharpe 都是**每根 K 线**的 `mean/std(ddof=1)`，不年化、无无风险利率。

```text
PSR(SR*) = Φ( (SR − SR*) × √(n−1) / √(1 − γ₃·SR + (γ₄ − 1)/4 · SR²) )
           γ₃ = m₃/m₂^1.5，γ₄ = m₄/m₂²（总体中心矩；γ₄ 是原始峰度，正态 = 3）
E[max SR] ≈ √V × ( (1−γ) Φ⁻¹(1 − 1/N) + γ Φ⁻¹(1 − 1/(N·e)) )，γ = 0.5772…（Euler–Mascheroni）
           N 次真实 Sharpe 为 0 的独立试验，仅凭运气会看到的最大 Sharpe；N = 1 → 0
DSR      = PSR(E[max SR])，N 与 V 来自全部试验的 Sharpe（非 NaN 个数与 ddof=1 方差）
PBO (CSCV)：
    行切成 S 个（≤ 16，偶数）近等长块；预计算每列每块的和与计数
    枚举所有 C(S, S/2) 种"取一半块做样本内"的划分：
        样本内赢家 = 样本内**平均收益**最高的列（不是 Sharpe：短块的 Sharpe 噪声大，零方差列会以 ∞ 取胜）
        它在样本外的平均排名 rank ∈ [1, n_cols]（1 最差，并列取平均）；ω = rank/(n_cols+1)；λ = ln(ω/(1−ω))
    PBO = #{λ ≤ 0} / #划分
```

## 数值细节

数值细节：`norm_cdf = ½ erfc(−x/√2)` 保住下尾的相对精度；`erfc` 在 `|z| < 2.5` 用全正项级数，否则用 Laplace 连分式（100 项）；`norm_ppf` 用 Acklam 有理逼近（相对误差 < 1.15e-9）再做一步 Halley 迭代，往返精度约 1e-13。

## 相关

- [phasor-analysis：分析层](../crates/phasor-analysis.md)：所属 crate；PSR、DSR、PBO 与 Acklam 逼近的原始论文列在它的[延伸资料](../crates/phasor-analysis.md#延伸资料)里。
- [指标公式表](../reference/metrics.md)：那里的 `sharpe_ratio` 年化并减去无风险利率，与这里每根 K 线的 Sharpe 不同。
- [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)：先汇总扫描结果、选出赢家，再用这里的检验问它是不是运气；两处都要防零方差列的 ∞。
- [门槛、多重比较与原创性检查](../algorithms/research-gates.md)：研究协议用 Bonferroni 门槛处理同一个多重比较问题的另一面。
- [练习与自测](../guides/exercises.md)：用纯噪声收益矩阵验证 DSR 接近 0.5、PBO 约 0.5 的练习。
