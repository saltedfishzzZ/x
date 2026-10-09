use crate::Vec3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CubeRotation(pub [[i8; 3]; 3]);

impl CubeRotation {
    #[must_use]
    pub fn apply(self, value: Vec3) -> Vec3 {
        let input = [value.x, value.y, value.z];
        let row = |r: usize| (0..3).map(|c| f64::from(self.0[r][c]) * input[c]).sum();
        Vec3::new(row(0), row(1), row(2))
    }
}

#[must_use]
pub fn cube_rotations() -> Vec<CubeRotation> {
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut result = Vec::with_capacity(24);
    for permutation in permutations {
        for sx in [-1_i8, 1] {
            for sy in [-1_i8, 1] {
                for sz in [-1_i8, 1] {
                    let mut matrix = [[0_i8; 3]; 3];
                    matrix[0][permutation[0]] = sx;
                    matrix[1][permutation[1]] = sy;
                    matrix[2][permutation[2]] = sz;
                    let determinant = i16::from(matrix[0][0])
                        * i16::from(matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
                        - i16::from(matrix[0][1])
                            * i16::from(matrix[1][0] * matrix[2][2] - matrix[1][2] * matrix[2][0])
                        + i16::from(matrix[0][2])
                            * i16::from(matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]);
                    if determinant == 1 {
                        result.push(CubeRotation(matrix));
                    }
                }
            }
        }
    }
    result
}
