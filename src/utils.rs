// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Helper module for functions that didn't fit anywhere else.

use image::{EncodableLayout, Primitive};
use nalgebra::base::dimension::{Dim, Dynamic};
use nalgebra::base::{Scalar, VecStorage};
use nalgebra::{DMatrix, Matrix};
use std::path::Path;

/// Same as rgb2gray matlab function, but for u8.
pub fn rgb_to_gray(red: &DMatrix<u8>, green: &DMatrix<u8>, blue: &DMatrix<u8>) -> DMatrix<u8> {
    let (rows, cols) = red.shape();
    DMatrix::from_iterator(
        rows,
        cols,
        red.iter()
            .zip(green.iter())
            .zip(blue.iter())
            .map(|((&r, &g), &b)| {
                (0.2989 * r as f32 + 0.5870 * g as f32 + 0.1140 * b as f32).max(255.0) as u8
            }),
    )
}

/// Reshapes `self` in-place such that it has dimensions `nrows × ncols`.
///
/// The values are not copied or moved. This function will panic if
/// provided dynamic sizes are not compatible.
pub fn reshape<N, R, C>(
    matrix: Matrix<N, R, C, VecStorage<N, R, C>>,
    nrows: usize,
    ncols: usize,
) -> DMatrix<N>
where
    N: Scalar,
    R: Dim,
    C: Dim,
{
    assert_eq!(nrows * ncols, matrix.data.len());
    let new_data = VecStorage::new(Dynamic::new(nrows), Dynamic::new(ncols), matrix.data.into());
    DMatrix::from_data(new_data)
}

/// Transpose a Vec of Vec.
/// Will crash if the inner vecs are not all the same size.
pub fn transpose<T: Clone>(v: Vec<Vec<T>>) -> Vec<Vec<T>> {
    // Checking case of an empty vec.
    if v.is_empty() {
        return Vec::new();
    }

    // Checking case of vec of empty vec.
    let transposed_len = v[0].len();
    assert!(v.iter().all(|vi| vi.len() == transposed_len));
    if transposed_len == 0 {
        return Vec::new();
    }

    // Normal case.
    let mut v_transposed = vec![Vec::new(); transposed_len];
    for vi in v.into_iter() {
        for (v_tj, vj) in v_transposed.iter_mut().zip(vi.into_iter()) {
            v_tj.push(vj);
        }
    }
    v_transposed
}

/// Save a bunch of gray images into the given directory.
pub fn save_imgs<P: AsRef<Path>, T: Scalar + Primitive>(dir: P, imgs: &[DMatrix<T>])
where
    [T]: EncodableLayout,
{
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).expect(&format!("Could not create output dir: {:?}", dir));
    imgs.iter().enumerate().for_each(|(i, img)| {
        crate::interop::image_from_matrix(img)
            .save(dir.join(format!("{}.png", i)))
            .expect("Error saving image");
    });
}

/// Save a bunch of RGB images into the given directory.
pub fn save_rgb_imgs<P: AsRef<Path>, T: Scalar + Primitive>(dir: P, imgs: &[DMatrix<(T, T, T)>])
where
    [T]: EncodableLayout,
{
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir).expect(&format!("Could not create output dir: {:?}", dir));
    imgs.iter().enumerate().for_each(|(i, img)| {
        crate::interop::rgb_from_matrix(img)
            .save(dir.join(format!("{}.png", i)))
            .expect("Error saving image");
    });
}

/// Retrieve the coordinates of selected pixels in a binary mask.
pub fn coordinates_from_mask(mask: &DMatrix<bool>) -> Vec<(usize, usize)> {
    crate::sparse::extract(mask.iter().cloned(), coords_col_major(mask.shape())).collect()
}

/// An iterator over all coordinates of a matrix in column major.
pub fn coords_col_major(shape: (usize, usize)) -> impl Iterator<Item = (usize, usize)> {
    let (height, width) = shape;
    let coords = (0..width).map(move |x| (0..height).map(move |y| (x, y)));
    coords.flatten()
}

/// An iterator over all coordinates of a matrix in row major.
pub fn coords_row_major(shape: (usize, usize)) -> impl Iterator<Item = (usize, usize)> {
    let (height, width) = shape;
    let coords = (0..height).map(move |y| (0..width).map(move |x| (x, y)));
    coords.flatten()
}
