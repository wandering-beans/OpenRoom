//! Point curves use a natural cubic spline; camera-profile curves retain monotone Hermite.
pub struct PointCurve<'a> {
    points: &'a [[f32; 2]],
    second: Vec<f32>,
}

impl<'a> PointCurve<'a> {
    pub fn new(points: &'a [[f32; 2]]) -> Self {
        let mut second = vec![0.; points.len()];
        let mut rhs = vec![0.; points.len()];
        for i in 1..points.len() - 1 {
            let left = points[i][0] - points[i - 1][0];
            let right = points[i + 1][0] - points[i][0];
            let span = left + right;
            let ratio = left / span;
            let pivot = ratio * second[i - 1] + 2.;
            second[i] = (ratio - 1.) / pivot;
            let slope_change = (points[i + 1][1] - points[i][1]) / right
                - (points[i][1] - points[i - 1][1]) / left;
            rhs[i] = (6. * slope_change / span - ratio * rhs[i - 1]) / pivot;
        }
        for i in (0..points.len() - 1).rev() {
            second[i] = second[i] * second[i + 1] + rhs[i];
        }
        Self { points, second }
    }

    pub fn sample(&self, x: f32) -> f32 {
        let points = self.points;
        let i = points
            .partition_point(|p| p[0] <= x)
            .saturating_sub(1)
            .min(points.len() - 2);
        let width = points[i + 1][0] - points[i][0];
        let b = ((x - points[i][0]) / width).clamp(0., 1.);
        let a = 1. - b;
        (a * points[i][1]
            + b * points[i + 1][1]
            + ((a * a * a - a) * self.second[i] + (b * b * b - b) * self.second[i + 1])
                * width
                * width
                / 6.)
            .clamp(0., 1.)
    }
}

pub fn sample(x: f32, points: &[[f32; 2]]) -> f32 {
    let last = points.len() - 1;
    let index = points
        .partition_point(|p| p[0] <= x)
        .saturating_sub(1)
        .min(last - 1);
    let slope = |i: usize| (points[i + 1][1] - points[i][1]) / (points[i + 1][0] - points[i][0]);
    let tangent = |i: usize| {
        if i == 0 {
            return slope(0);
        }
        if i == last {
            return slope(last - 1);
        }
        let (left, right) = (slope(i - 1), slope(i));
        if left * right <= 0. {
            return 0.;
        }
        let a = points[i][0] - points[i - 1][0];
        let b = points[i + 1][0] - points[i][0];
        let (w1, w2) = (2. * b + a, b + 2. * a);
        (w1 + w2) / (w1 / left + w2 / right)
    };
    let width = points[index + 1][0] - points[index][0];
    let t = ((x - points[index][0]) / width).clamp(0., 1.);
    let t2 = t * t;
    let t3 = t2 * t;
    ((2. * t3 - 3. * t2 + 1.) * points[index][1]
        + (t3 - 2. * t2 + t) * width * tangent(index)
        + (-2. * t3 + 3. * t2) * points[index + 1][1]
        + (t3 - t2) * width * tangent(index + 1))
    .clamp(0., 1.)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smooth_curve_hits_points_and_does_not_overshoot() {
        let points = [[0., 0.], [0.2, 0.1], [0.6, 0.8], [1., 1.]];
        for point in points {
            assert!((sample(point[0], &points) - point[1]).abs() < 1e-6);
        }
        let values: Vec<_> = (0..=1000)
            .map(|i| sample(i as f32 / 1000., &points))
            .collect();
        assert!(values.windows(2).all(|p| p[0] <= p[1]));
        let epsilon = 0.0001;
        for x in [0.2, 0.6] {
            let left = (sample(x, &points) - sample(x - epsilon, &points)) / epsilon;
            let right = (sample(x + epsilon, &points) - sample(x, &points)) / epsilon;
            assert!((left - right).abs() < 0.01);
        }
    }
}
