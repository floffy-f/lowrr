// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Registration algorithm for a sequence of slightly misaligned images.

use nalgebra::{DMatrix, Matrix3, Matrix6, RealField, Vector2, Vector3, Vector6};

/// Configuration (parameters) of the registration algorithm.
#[derive(Debug)]
pub struct Config {
    pub do_image_correction: bool,
    pub lambda: f32,
    pub rho: f32,
    pub max_iterations: usize,
    pub threshold: f32,
    pub image_max: f32,
    pub levels: usize,
    pub trace: bool,
}

/// Type alias just to visually differenciate Vec<Vec<_>>
/// when it is Vec<Levels<_>> or Levels<Vec<_>>.
type Levels<T> = Vec<T>;

/// Registration of single channel images.
///
/// Internally, this uses a multi-resolution approach,
/// where the motion vector computed at one resolution serves
/// as initialization for the next one.
pub fn gray_images(
    config: Config,
    imgs: Vec<DMatrix<u8>>,
) -> Result<(Vec<Vector6<f32>>, Vec<DMatrix<u8>>), Box<dyn std::error::Error>> {
    // Get the number of images to align.
    let imgs_count = imgs.len();

    // Precompute a hierarchy of multi-resolution images and gradients norm.
    let mut multires_imgs: Vec<Levels<_>> = Vec::with_capacity(imgs_count);
    let mut multires_gradient_sqr_norm: Vec<Levels<_>> = Vec::with_capacity(imgs_count);
    for im in imgs.into_iter() {
        let pyramid = crate::multires::mean_pyramid(config.levels, im);
        let mut gradients = Vec::with_capacity(config.levels);
        for lvl_im in pyramid.iter() {
            gradients.push(crate::gradients::squared_norm_direct(lvl_im));
        }
        multires_gradient_sqr_norm.push(gradients);
        multires_imgs.push(pyramid);
    }

    // Transpose the `Vec<Levels<_>>` structure of multires images and gradients
    // into a `Levels<Vec<_>>` to have each level regrouped.
    let multires_imgs: Levels<Vec<_>> = crate::utils::transpose(multires_imgs);
    let multires_gradient_sqr_norm: Levels<Vec<_>> =
        crate::utils::transpose(multires_gradient_sqr_norm);

    // Initialize the motion vector.
    let mut motion_vec = vec![Vector6::zeros(); imgs_count];

    // Multi-resolution algorithm.
    // Does the same thing at each level for the corresponding images and gradients.
    // The iterator is reversed to start at last level (lowest resolution).
    // Level 0 are the initial images.
    for (level, l_imgs) in multires_imgs.iter().enumerate().rev() {
        eprintln!("\n=============  Start level {}  =============\n", level);

        // Algorithm parameters.
        let (height, width) = l_imgs[0].shape();
        let step_config = StepConfig {
            do_image_correction: config.do_image_correction,
            lambda: config.lambda,
            rho: config.rho,
            max_iterations: config.max_iterations,
            threshold: config.threshold,
            debug_trace: config.trace,
        };

        // motion_vec is adapted when changing level.
        for motion in motion_vec.iter_mut() {
            motion[4] = 2.0 * motion[4];
            motion[5] = 2.0 * motion[5];
        }

        // We also recompute the registered images before starting the algorithm loop.
        let pixels_count = height * width;
        let mut imgs_registered = DMatrix::zeros(pixels_count, imgs_count);
        project_f32(width, height, &mut imgs_registered, &l_imgs, &motion_vec);
        let compute_registered_gradients =
            |i| compute_registered_gradients_full((height, width), i, &imgs_registered);

        // Updated state variables for the loops.
        let mut loop_state = State {
            nb_iter: 0,
            imgs_registered,
            old_imgs_a: DMatrix::zeros(pixels_count, imgs_count),
            errors: DMatrix::zeros(pixels_count, imgs_count),
            lagrange_mult_rho: DMatrix::zeros(pixels_count, imgs_count),
            motion_vec: motion_vec.clone(),
            compute_registered_gradients,
        };
        let obs = Obs {
            image_size: (width, height),
            images: l_imgs.as_slice(),
        };

        // Main loop.
        let mut continuation = Continue::Forward;
        while continuation == Continue::Forward {
            let (new_state, new_continuation) = step(&step_config, &obs, loop_state);
            loop_state = new_state;
            continuation = new_continuation;
        }

        // Update the motion vec before next level
        motion_vec = loop_state.motion_vec;
        eprintln!("motion_vec:");
        motion_vec.iter().for_each(|v| eprintln!("   {:?}", v.data));
    } // End of levels

    // Return the final motion vector.
    // And give back the images at original resolution.
    let imgs = multires_imgs.into_iter().next().unwrap();
    Ok((motion_vec, imgs))
}

/// Configuration parameters for the core loop of the algorithm.
struct StepConfig {
    do_image_correction: bool,
    lambda: f32,
    rho: f32,
    max_iterations: usize,
    threshold: f32,
    debug_trace: bool,
}

/// "Observations" contains the data provided outside the core of the algorithm.
/// These are immutable references since we are not supposed to mutate them.
struct Obs<'a> {
    image_size: (usize, usize),
    images: &'a [DMatrix<u8>],
}

/// Simple enum type to indicate if we should continue to loop.
/// This is to avoid the ambiguity of booleans.
#[derive(PartialEq)]
enum Continue {
    Forward,
    Stop,
}

/// State variables of the loop.
struct State<F: Fn(usize) -> DMatrix<(f32, f32)>> {
    nb_iter: usize,
    imgs_registered: DMatrix<f32>,   // W(u; theta) in paper
    old_imgs_a: DMatrix<f32>,        // A in paper
    errors: DMatrix<f32>,            // e in paper
    lagrange_mult_rho: DMatrix<f32>, // y / rho in paper
    motion_vec: Vec<Vector6<f32>>,   // theta in paper
    compute_registered_gradients: F,
}

/// Core iteration step of the algorithm.
fn step<F: Fn(usize) -> DMatrix<(f32, f32)>>(
    config: &StepConfig,
    obs: &Obs,
    state: State<F>,
) -> (State<F>, Continue) {
    // Extract state variables to avoid prefixed notation later.
    let (width, height) = obs.image_size;
    let State {
        nb_iter,
        old_imgs_a,
        mut imgs_registered,
        mut errors,
        mut lagrange_mult_rho,
        mut motion_vec,
        mut compute_registered_gradients,
    } = state;
    let lambda = config.lambda / (imgs_registered.nrows() as f32).sqrt();

    // A-update: low-rank approximation
    let imgs_a_temp = &imgs_registered + &errors + &lagrange_mult_rho;
    let mut svd = imgs_a_temp.svd(true, true);
    for x in svd.singular_values.iter_mut() {
        *x = shrink(1.0 / config.rho, *x);
    }
    let singular_values = svd.singular_values.clone();
    let imgs_a = svd.recompose().unwrap();

    // e-update: L1-regularized least-squares
    let errors_temp = &imgs_a - &imgs_registered - &lagrange_mult_rho;
    if config.do_image_correction {
        errors = errors_temp.map(|x| shrink(lambda / config.rho, x));
    }

    // theta-update: forwards compositional step of a Gauss-Newton approximation.
    let residuals = &errors_temp - &errors;
    for i in 0..obs.images.len() {
        // Compute residuals and motion step,
        let gradients = compute_registered_gradients(i);
        let coordinates = (0..width).map(|x| (0..height).map(move |y| (x, y)));
        let step_params = forwards_compositional_step(
            (height, width),
            coordinates.flatten(),
            residuals.column(i).iter().cloned(),
            gradients.iter().cloned(),
        );

        // Save motion for this image.
        motion_vec[i] =
            projection_params(&(projection_mat(&motion_vec[i]) * projection_mat(&step_params)));
    }

    // Transform all motion parameters such that image 0 is the reference.
    let inverse_motion_ref = projection_mat(&motion_vec[0])
        .try_inverse()
        .expect("Error while inversing motion of reference image");
    for motion_params in motion_vec.iter_mut() {
        *motion_params = projection_params(&(inverse_motion_ref * projection_mat(&motion_params)));
    }

    // Update imgs_registered.
    project_f32(
        width,
        height,
        &mut imgs_registered,
        &obs.images,
        &motion_vec,
    );

    // w-update: dual ascent
    lagrange_mult_rho += &imgs_registered - &imgs_a + &errors;

    // Update the registered gradients computation.
    compute_registered_gradients =
        |i| compute_registered_gradients_full((height, width), i, &imgs_registered);

    // Check convergence
    let residual = norm(&(&imgs_a - &old_imgs_a)) / 1e-12.max(norm(&old_imgs_a));
    if config.debug_trace {
        let nuclear_norm = singular_values.sum();
        let l1_norm = lambda * errors.map(|x| x.abs()).sum();
        let r = &imgs_registered - &imgs_a + &errors;
        let augmented_lagrangian = nuclear_norm
            + l1_norm
            + config.rho * (lagrange_mult_rho.component_mul(&r)).sum()
            + 0.5 * config.rho * (norm_sqr(&r) as f32);
        eprintln!("");
        eprintln!("Iteration {}:", nb_iter);
        eprintln!("    Nucl norm: {}", nuclear_norm);
        eprintln!("    L1 norm: {}", l1_norm);
        eprintln!("    Nucl + L1: {}", l1_norm + nuclear_norm);
        eprintln!("    Aug. Lagrangian: {}", augmented_lagrangian);
        eprintln!("    residual: {}", residual);
        eprintln!("");
    }
    let mut continuation = Continue::Forward;
    if nb_iter >= config.max_iterations || residual < config.threshold {
        continuation = Continue::Stop;
    }

    // Returned value
    (
        State {
            nb_iter: nb_iter + 1,
            imgs_registered,
            old_imgs_a: imgs_a,
            errors,
            lagrange_mult_rho,
            motion_vec,
            compute_registered_gradients,
        },
        continuation,
    )
}

fn compute_registered_gradients_full(
    shape: (usize, usize),
    i: usize,
    imgs_registered: &DMatrix<f32>,
) -> DMatrix<(f32, f32)> {
    let (nrows, ncols) = shape;
    let img_registered_i = DMatrix::from_columns(&[imgs_registered.column(i)]);
    let img_registered_i_shaped = crate::utils::reshape(img_registered_i, nrows, ncols);
    crate::gradients::centered_f32(&img_registered_i_shaped)
}

fn forwards_compositional_step(
    shape: (usize, usize),
    coordinates: impl Iterator<Item = (usize, usize)>,
    residuals: impl Iterator<Item = f32>,
    gradients: impl Iterator<Item = (f32, f32)>,
) -> Vector6<f32> {
    let (height, width) = shape;
    let mut descent_params = Vector6::zeros();
    let mut hessian = Matrix6::zeros();
    let border = (0.04 * height.min(width) as f32) as usize;
    for (((x, y), res), (gx, gy)) in coordinates.zip(residuals).zip(gradients) {
        // Only use points within a given margin.
        if x > border && x + border < width && y > border && y + border < height {
            let x_ = x as f32;
            let y_ = y as f32;
            let jac_t = Vector6::new(x_ * gx, x_ * gy, y_ * gx, y_ * gy, gx, gy);
            hessian += jac_t * jac_t.transpose();
            descent_params += res * jac_t;
        }
    }
    let hessian_chol = hessian.cholesky().expect("Error hessian choleski");
    hessian_chol.solve(&descent_params)
}

#[rustfmt::skip]
pub fn projection_mat(params: &Vector6<f32>) -> Matrix3<f32> {
    Matrix3::new(
        1.0 + params[0], params[2], params[4],
        params[1], 1.0 + params[3], params[5],
        0.0, 0.0, 1.0,
    )
}

pub fn projection_params(mat: &Matrix3<f32>) -> Vector6<f32> {
    Vector6::new(
        mat.m11 - 1.0,
        mat.m21,
        mat.m12,
        mat.m22 - 1.0,
        mat.m13,
        mat.m23,
    )
}

/// Compute the projection of each pixel of the image (modify in place).
fn project_f32(
    width: usize,
    height: usize,
    registered: &mut DMatrix<f32>,
    imgs: &[DMatrix<u8>],
    motion_vec: &[Vector6<f32>],
) {
    let inv_max = 1.0 / 255.0;
    for (i, motion) in motion_vec.iter().enumerate() {
        let motion_mat = projection_mat(motion);
        let mut idx = 0;
        for x in 0..width {
            for y in 0..height {
                let new_pos = motion_mat * Vector3::new(x as f32, y as f32, 1.0);
                registered[(idx, i)] =
                    inv_max * crate::interpolation::linear(new_pos.x, new_pos.y, &imgs[i]);
                idx += 1;
            }
        }
    }
}

/// Compute the projection of each pixel of the image.
/// Outputs a grayscale image (0-255).
pub fn reproject_u8(imgs: &[DMatrix<u8>], motion_vec: &[Vector6<f32>]) -> Vec<DMatrix<u8>> {
    let (height, width) = imgs[0].shape();
    let mut all_registered = Vec::new();
    for (im, motion) in imgs.iter().zip(motion_vec.iter()) {
        let motion_mat = projection_mat(motion);
        let registered = DMatrix::from_fn(height, width, |i, j| {
            let new_pos = motion_mat * Vector3::new(j as f32, i as f32, 1.0);
            crate::interpolation::linear(new_pos.x, new_pos.y, im)
                .max(0.0)
                .min(255.0) as u8
        });
        all_registered.push(registered);
    }
    all_registered
}

/// Computes the sqrt of the sum of squared values.
/// This is the L2 norm of the vectorized version of the matrix.
fn norm(matrix: &DMatrix<f32>) -> f32 {
    norm_sqr(matrix).sqrt() as f32
}

fn norm_sqr(matrix: &DMatrix<f32>) -> f64 {
    matrix.iter().map(|&x| (x as f64).powi(2)).sum()
}

/// Shrink values toward 0.
fn shrink<T: RealField>(alpha: T, x: T) -> T {
    let alpha = alpha.abs();
    if x.is_sign_positive() {
        (x - alpha).max(T::zero())
    } else {
        (x + alpha).min(T::zero())
    }
}
