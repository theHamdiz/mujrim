//! Training compute backends. Phase 4 ships a CPU matvec; GPU kernels plug in later.

pub trait TrainCompute: Send + Sync {
    fn matvec_f32(&self, matrix: &[f32], vector: &[f32], rows: usize, cols: usize, out: &mut [f32]);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CpuCompute;

impl TrainCompute for CpuCompute {
    fn matvec_f32(
        &self,
        matrix: &[f32],
        vector: &[f32],
        rows: usize,
        cols: usize,
        out: &mut [f32],
    ) {
        assert_eq!(matrix.len(), rows * cols);
        assert_eq!(vector.len(), cols);
        assert_eq!(out.len(), rows);
        for (row, dst) in out.iter_mut().enumerate() {
            let start = row * cols;
            let mut sum = 0.0;
            for (&weight, &activation) in matrix[start..start + cols].iter().zip(vector) {
                sum += weight * activation;
            }
            *dst = sum;
        }
    }
}

/// Pick the training compute backend. Phase 4 always returns CPU; detection
/// stays available for later GPU kernels.
pub fn training_compute() -> CpuCompute {
    CpuCompute
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_matvec_matches_naive_dot_products() {
        let matrix = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let vector = [2.0, 0.5, -1.0];
        let mut out = [0.0; 2];
        CpuCompute.matvec_f32(&matrix, &vector, 2, 3, &mut out);
        assert_eq!(out[0], 1.0 * 2.0 + 2.0 * 0.5 - 3.0);
        assert_eq!(out[1], 4.0 * 2.0 + 5.0 * 0.5 - 6.0);
    }

    #[test]
    fn training_compute_is_the_cpu_backend() {
        let _ = training_compute();
        let info = crate::system_info();
        assert!(!info.os.is_empty());
    }

    #[test]
    fn cpu_matvec_stays_within_a_latency_budget() {
        let rows = 64;
        let cols = 256;
        let matrix = vec![0.125f32; rows * cols];
        let vector = vec![0.5f32; cols];
        let mut out = vec![0.0f32; rows];
        let start = std::time::Instant::now();
        for _ in 0..64 {
            CpuCompute.matvec_f32(&matrix, &vector, rows, cols, &mut out);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "CPU matvec budget exceeded: {elapsed:?}"
        );
        assert!(out.iter().all(|value| value.is_finite()));
    }
}
