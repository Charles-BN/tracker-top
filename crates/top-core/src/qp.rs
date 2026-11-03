//! A small dense convex QP solver, sized for tracker problems (tens of variables).
//!
//! ```text
//! minimise    0.5 x'Px + q'x
//! subject to  l <= Ax <= u
//! ```
//!
//! The algorithm is the ADMM scheme used by OSQP (Stellato et al., 2020) with a
//! dense Cholesky factorisation and adaptive step size. Matrices are row-major
//! `Vec<f64>`. For anything much larger than a few hundred variables, plug a
//! sparse solver in behind the [`QpSolver`] trait instead.

/// QP in "OSQP form". `p` is the full symmetric `n x n` matrix, `a` is `m x n`.
/// Use `f64::NEG_INFINITY` / `f64::INFINITY` for one-sided rows.
#[derive(Clone, Debug)]
pub struct Qp {
    pub n: usize,
    pub m: usize,
    pub p: Vec<f64>,
    pub q: Vec<f64>,
    pub a: Vec<f64>,
    pub l: Vec<f64>,
    pub u: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QpStatus {
    Solved,
    /// Iteration limit reached before the tolerances were met.
    MaxIterations,
    /// The KKT matrix could not be factorised.
    NumericalFailure,
}

#[derive(Clone, Debug)]
pub struct QpSolution {
    pub x: Vec<f64>,
    pub status: QpStatus,
    pub iterations: usize,
    /// max |Ax - z| at exit (constraint violation measure).
    pub primal_residual: f64,
    pub dual_residual: f64,
}

pub trait QpSolver: Sync {
    fn solve(&self, qp: &Qp) -> QpSolution;
}

#[derive(Clone, Debug)]
pub struct AdmmSolver {
    pub max_iter: usize,
    pub eps_abs: f64,
    pub eps_rel: f64,
    pub sigma: f64,
    pub alpha: f64,
    pub rho: f64,
    /// Residuals (and adaptive rho) are evaluated every this many iterations.
    pub check_every: usize,
}

impl Default for AdmmSolver {
    fn default() -> Self {
        Self {
            max_iter: 100_000,
            eps_abs: 1e-9,
            eps_rel: 1e-9,
            sigma: 1e-6,
            alpha: 1.6,
            rho: 0.1,
            check_every: 10,
        }
    }
}

fn inf_norm(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// In-place lower Cholesky factor of a symmetric positive definite `n x n` matrix.
fn cholesky(k: &mut [f64], n: usize) -> bool {
    for j in 0..n {
        let mut d = k[j * n + j];
        for s in 0..j {
            d -= k[j * n + s] * k[j * n + s];
        }
        if !(d > 0.0) {
            return false;
        }
        let d = d.sqrt();
        k[j * n + j] = d;
        for i in (j + 1)..n {
            let mut v = k[i * n + j];
            for s in 0..j {
                v -= k[i * n + s] * k[j * n + s];
            }
            k[i * n + j] = v / d;
        }
    }
    true
}

fn chol_solve(l: &[f64], n: usize, b: &mut [f64]) {
    for i in 0..n {
        let mut v = b[i];
        for s in 0..i {
            v -= l[i * n + s] * b[s];
        }
        b[i] = v / l[i * n + i];
    }
    for i in (0..n).rev() {
        let mut v = b[i];
        for s in (i + 1)..n {
            v -= l[s * n + i] * b[s];
        }
        b[i] = v / l[i * n + i];
    }
}

fn factor(qp: &Qp, ata: &[f64], sigma: f64, rho: f64) -> Option<Vec<f64>> {
    let n = qp.n;
    let mut k = vec![0.0; n * n];
    for i in 0..n * n {
        k[i] = qp.p[i] + rho * ata[i];
    }
    for i in 0..n {
        k[i * n + i] += sigma;
    }
    cholesky(&mut k, n).then_some(k)
}

fn mat_vec(a: &[f64], rows: usize, cols: usize, x: &[f64], out: &mut [f64]) {
    for r in 0..rows {
        let row = &a[r * cols..(r + 1) * cols];
        out[r] = row.iter().zip(x).map(|(a, b)| a * b).sum();
    }
}

fn mat_t_vec(a: &[f64], rows: usize, cols: usize, y: &[f64], out: &mut [f64]) {
    out.iter_mut().for_each(|v| *v = 0.0);
    for r in 0..rows {
        let w = y[r];
        if w == 0.0 {
            continue;
        }
        let row = &a[r * cols..(r + 1) * cols];
        for i in 0..cols {
            out[i] += row[i] * w;
        }
    }
}

impl QpSolver for AdmmSolver {
    fn solve(&self, qp: &Qp) -> QpSolution {
        let (n, m) = (qp.n, qp.m);
        debug_assert_eq!(qp.p.len(), n * n);
        debug_assert_eq!(qp.a.len(), m * n);

        // A'A, reused whenever rho changes.
        let mut ata = vec![0.0; n * n];
        for r in 0..m {
            let row = &qp.a[r * n..(r + 1) * n];
            for i in 0..n {
                if row[i] == 0.0 {
                    continue;
                }
                for j in 0..n {
                    ata[i * n + j] += row[i] * row[j];
                }
            }
        }

        let mut rho = self.rho;
        let Some(mut chol) = factor(qp, &ata, self.sigma, rho) else {
            return QpSolution {
                x: vec![0.0; n],
                status: QpStatus::NumericalFailure,
                iterations: 0,
                primal_residual: f64::INFINITY,
                dual_residual: f64::INFINITY,
            };
        };

        let mut x = vec![0.0; n];
        let mut z: Vec<f64> = (0..m).map(|i| 0.0f64.max(qp.l[i]).min(qp.u[i])).collect();
        let mut y = vec![0.0; m];

        let mut rhs = vec![0.0; n];
        let mut xt = vec![0.0; n];
        let mut zt = vec![0.0; m];
        let mut w = vec![0.0; m];
        let mut ax = vec![0.0; m];
        let mut px = vec![0.0; n];
        let mut aty = vec![0.0; n];
        let mut r_prim = f64::INFINITY;
        let mut r_dual = f64::INFINITY;
        let mut status = QpStatus::MaxIterations;
        let mut iterations = 0;

        for k in 1..=self.max_iter {
            iterations = k;

            // x-update: (P + sigma I + rho A'A) xt = sigma x - q + A'(rho z - y)
            for r in 0..m {
                w[r] = rho * z[r] - y[r];
            }
            mat_t_vec(&qp.a, m, n, &w, &mut rhs);
            for i in 0..n {
                rhs[i] += self.sigma * x[i] - qp.q[i];
            }
            xt.copy_from_slice(&rhs);
            chol_solve(&chol, n, &mut xt);
            mat_vec(&qp.a, m, n, &xt, &mut zt);

            // Over-relaxation, projection and dual update.
            for i in 0..n {
                x[i] = self.alpha * xt[i] + (1.0 - self.alpha) * x[i];
            }
            for r in 0..m {
                let zr = self.alpha * zt[r] + (1.0 - self.alpha) * z[r];
                let zn = (zr + y[r] / rho).max(qp.l[r]).min(qp.u[r]);
                y[r] += rho * (zr - zn);
                z[r] = zn;
            }

            if k % self.check_every != 0 && k != self.max_iter {
                continue;
            }

            mat_vec(&qp.a, m, n, &x, &mut ax);
            mat_vec(&qp.p, n, n, &x, &mut px);
            mat_t_vec(&qp.a, m, n, &y, &mut aty);
            r_prim = ax.iter().zip(&z).fold(0.0, |s, (a, b)| s.max((a - b).abs()));
            r_dual = (0..n).fold(0.0, |s, i| s.max((px[i] + qp.q[i] + aty[i]).abs()));

            let prim_scale = inf_norm(&ax).max(inf_norm(&z));
            let dual_scale = inf_norm(&px).max(inf_norm(&aty)).max(inf_norm(&qp.q));
            let eps_prim = self.eps_abs + self.eps_rel * prim_scale;
            let eps_dual = self.eps_abs + self.eps_rel * dual_scale;
            if r_prim <= eps_prim && r_dual <= eps_dual {
                status = QpStatus::Solved;
                break;
            }

            // Adaptive rho: balance the normalised primal and dual residuals.
            let num = r_prim / prim_scale.max(1e-10);
            let den = r_dual / dual_scale.max(1e-10);
            if num > 0.0 && den > 0.0 {
                let new_rho = (rho * (num / den).sqrt()).clamp(1e-6, 1e6);
                if new_rho > 5.0 * rho || new_rho < rho / 5.0 {
                    match factor(qp, &ata, self.sigma, new_rho) {
                        Some(c) => {
                            chol = c;
                            rho = new_rho;
                        }
                        None => {
                            status = QpStatus::NumericalFailure;
                            break;
                        }
                    }
                }
            }
        }

        QpSolution {
            x,
            status,
            iterations,
            primal_residual: r_prim,
            dual_residual: r_dual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn projection_onto_half_plane() {
        // min (x1-1)^2 + (x2-2)^2  s.t. x1 + x2 <= 1   ->   x = (0, 1)
        let qp = Qp {
            n: 2,
            m: 1,
            p: vec![2.0, 0.0, 0.0, 2.0],
            q: vec![-2.0, -4.0],
            a: vec![1.0, 1.0],
            l: vec![f64::NEG_INFINITY],
            u: vec![1.0],
        };
        let s = AdmmSolver::default().solve(&qp);
        assert_eq!(s.status, QpStatus::Solved);
        assert!(close(s.x[0], 0.0, 1e-6), "{:?}", s.x);
        assert!(close(s.x[1], 1.0, 1e-6), "{:?}", s.x);
    }

    #[test]
    fn box_constraints_clip_the_unconstrained_minimum() {
        // min (x-5)^2 + (y+5)^2  s.t. 0 <= x <= 1, -2 <= y <= 2  ->  (1, -2)
        let qp = Qp {
            n: 2,
            m: 2,
            p: vec![2.0, 0.0, 0.0, 2.0],
            q: vec![-10.0, 10.0],
            a: vec![1.0, 0.0, 0.0, 1.0],
            l: vec![0.0, -2.0],
            u: vec![1.0, 2.0],
        };
        let s = AdmmSolver::default().solve(&qp);
        assert_eq!(s.status, QpStatus::Solved);
        assert!(close(s.x[0], 1.0, 1e-6) && close(s.x[1], -2.0, 1e-6), "{:?}", s.x);
    }

    #[test]
    fn two_sided_row_with_interior_solution() {
        // min x^2 + y^2 - 2x - 2y  s.t. -10 <= x + y <= 10  -> (1, 1), constraint inactive
        let qp = Qp {
            n: 2,
            m: 1,
            p: vec![2.0, 0.0, 0.0, 2.0],
            q: vec![-2.0, -2.0],
            a: vec![1.0, 1.0],
            l: vec![-10.0],
            u: vec![10.0],
        };
        let s = AdmmSolver::default().solve(&qp);
        assert_eq!(s.status, QpStatus::Solved);
        assert!(close(s.x[0], 1.0, 1e-6) && close(s.x[1], 1.0, 1e-6), "{:?}", s.x);
    }
}
