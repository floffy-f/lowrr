use lowrr::img::crop::{crop, recover_original_motion, Crop};
use lowrr::img::interpolation::CanLinearInterpolate;
use lowrr::img::registration::{self, CanRegister};
use lowrr::interop::{IntoDMatrix, IntoImage};
use lowrr::utils::CanEqualize;

use anyhow::Context;
use glob::glob;
use image::DynamicImage;
use nalgebra::{DMatrix, Scalar, Vector6};
use std::convert::TryFrom;
use std::ops::{Add, Mul};
use std::path::{Path, PathBuf};

// Default values for some of the program arguments.
const DEFAULT_OUT_DIR: &str = "out";

const DEFAULT_LEVELS: &str = "4";
const DEFAULT_SPARSE_RATIO_THRESHOLD: &str = "0.5";

const DEFAULT_LAMBDA: &str = "1.5";
const DEFAULT_RHO: &str = "0.1";

const DEFAULT_THRESHOLD: &str = "1e-3";
const DEFAULT_MAX_ITERATIONS: &str = "40";

/// Entry point of the program.
fn main() -> anyhow::Result<()> {
    // CLI arguments related to the core parameters of the algorithm.
    let core_args = vec![
        clap::Arg::with_name("lambda")
            .long("lambda")
            .value_name("x")
            .default_value(DEFAULT_LAMBDA)
            .help("Weight of the L1 term (high means no correction)"),
        clap::Arg::with_name("rho")
            .long("rho")
            .value_name("x")
            .default_value(DEFAULT_RHO)
            .help("Lagrangian penalty"),
        clap::Arg::with_name("convergence-threshold")
            .long("convergence-threshold")
            .value_name("x")
            .default_value(DEFAULT_THRESHOLD)
            .help(
                "Stop when relative diff between two estimate of corrected image falls below this",
            ),
        clap::Arg::with_name("max-iterations")
            .long("max-iterations")
            .default_value(DEFAULT_MAX_ITERATIONS)
            .value_name("N")
            .help("Maximum number of iterations"),
    ];
    // CLI arguments related to algorithm speedup techniques.
    let speed_args = vec![
        clap::Arg::with_name("crop")
            .long("crop")
            .number_of_values(4)
            .value_names(&["left", "top", "right", "bottom"])
            .use_delimiter(true)
            .help("Crop image into a restricted working area"),
        clap::Arg::with_name("levels")
            .long("levels")
            .default_value(DEFAULT_LEVELS)
            .value_name("N")
            .help("Number of levels for the multi-resolution approach"),
        clap::Arg::with_name("sparse-switch")
            .long("sparse-switch")
            .value_name("ratio")
            .default_value(DEFAULT_SPARSE_RATIO_THRESHOLD)
            .help("Sparse ratio threshold to switch between dense and sparse resolution. Use dense resolution if the ratio at current level is higher than this threshold"),
    ];
    // CLI arguments related to input, output and the rest.
    let input_output_args = vec![
        clap::Arg::with_name("out-dir")
            .long("out-dir")
            .default_value(DEFAULT_OUT_DIR)
            .value_name("path")
            .help("Output directory to save registered images"),
        clap::Arg::with_name("trace")
            .long("trace")
            .help("Print more debug output to stderr while running"),
        clap::Arg::with_name("IMAGE or GLOB")
            .multiple(true)
            .required(true)
            .help("Paths to images, or glob pattern such as \"img/*.png\""),
    ];
    // Read all CLI arguments.
    let matches = clap::App::new("lowrr")
        .version(std::env!("CARGO_PKG_VERSION"))
        .about("Low-rank registration of slightly misaligned images for photometric stereo")
        .args(&core_args)
        .args(&speed_args)
        .args(&input_output_args)
        .get_matches();
    run(get_args(&matches)?)
}

#[derive(Debug)]
/// Type holding command line arguments.
struct Args {
    config: registration::Config,
    out_dir: String,
    images_paths: Vec<PathBuf>,
    crop: Option<Crop>,
}

/// Retrieve the program arguments from clap matches.
fn get_args(matches: &clap::ArgMatches) -> anyhow::Result<Args> {
    let config = registration::Config {
        trace: matches.is_present("trace"),
        lambda: matches.value_of("lambda").unwrap().parse()?,
        rho: matches.value_of("rho").unwrap().parse()?,
        threshold: matches.value_of("convergence-threshold").unwrap().parse()?,
        sparse_ratio_threshold: matches.value_of("sparse-switch").unwrap().parse()?,
        max_iterations: matches.value_of("max-iterations").unwrap().parse()?,
        levels: matches.value_of("levels").unwrap().parse()?,
    };

    let crop = match matches.values_of("crop") {
        None => None,
        Some(str_coords) => Some(Crop::try_from(str_coords)?),
    };

    Ok(Args {
        config,
        out_dir: matches.value_of("out-dir").unwrap().to_string(),
        images_paths: absolute_file_paths(matches.values_of("IMAGE or GLOB").unwrap())?,
        crop,
    })
}

/// Retrieve the absolute paths of all files matching the arguments.
fn absolute_file_paths<S: AsRef<str>, Paths: Iterator<Item = S>>(
    args: Paths,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut abs_paths = Vec::new();
    for path_glob in args {
        let mut paths = paths_from_glob(path_glob.as_ref())?;
        abs_paths.append(&mut paths);
    }
    abs_paths
        .iter()
        .map(|p| p.canonicalize().map_err(|e| e.into()))
        .collect()
}

/// Retrieve the paths of files matchin the glob pattern.
fn paths_from_glob(p: &str) -> anyhow::Result<Vec<PathBuf>> {
    let paths = glob(p)?;
    Ok(paths.into_iter().filter_map(|x| x.ok()).collect())
}

/// Start actual program with command line arguments successfully parsed.
fn run(args: Args) -> anyhow::Result<()> {
    // Load the dataset in memory.
    let now = std::time::Instant::now();
    let (dataset, _) = load_dataset(&args.images_paths)?;
    eprintln!("Loading took {:.1} s", now.elapsed().as_secs_f32());

    // Use the algorithm corresponding to the type of data.
    let motion_vec = match dataset {
        Dataset::GrayImages(_) => unimplemented!(),
        Dataset::RgbImages(imgs) => {
            let gray_imgs: Vec<_> = imgs.iter().map(|im| im.map(|(_r, g, _b)| g)).collect();
            let (motion_vec_crop, cropped_eq_imgs) =
                crop_and_register(args.crop, args.config, gray_imgs, 40)?;
            original_motion(&args, motion_vec_crop, cropped_eq_imgs, &imgs)?
        }
        Dataset::RgbImagesU16(imgs) => {
            let gray_imgs: Vec<_> = imgs.iter().map(|im| im.map(|(_r, g, _b)| g)).collect();
            let (motion_vec_crop, cropped_eq_imgs) =
                crop_and_register(args.crop, args.config, gray_imgs, 10 * 256)?;
            original_motion(&args, motion_vec_crop, cropped_eq_imgs, &imgs)?
        }
        Dataset::RawImages(_) => unimplemented!(),
    };

    // Write motion_vec to stdout.
    for v in motion_vec.iter() {
        println!("{}", v);
    }
    Ok(())
}

fn crop_and_register<T: CanEqualize + CanRegister>(
    args_crop: Option<Crop>,
    registration_config: registration::Config,
    gray_imgs: Vec<DMatrix<T>>,
    sparse_diff_threshold: <T as CanRegister>::Bigger, // 50
) -> anyhow::Result<(Vec<Vector6<f32>>, Vec<DMatrix<T>>)>
where
    DMatrix<T>: IntoImage,
{
    // Extract the cropped area from the images.
    let mut cropped_imgs = match args_crop {
        None => gray_imgs,
        Some(frame) => gray_imgs.iter().map(|im| crop(frame, im)).collect(),
    };

    // Equalize mean intensities of cropped area.
    lowrr::utils::equalize_mean(0.15, &mut cropped_imgs);

    // Compute the motion of each image for registration.
    Ok(
        registration::gray_affine(registration_config, cropped_imgs, sparse_diff_threshold)
            .unwrap(),
    )
}

fn original_motion<T: CanRegister, U: Scalar + Copy, V>(
    args: &Args,
    motion_vec_crop: Vec<Vector6<f32>>,
    cropped_eq_imgs: Vec<DMatrix<T>>,
    original_imgs: &[DMatrix<U>],
) -> anyhow::Result<Vec<Vector6<f32>>>
where
    DMatrix<T>: IntoImage,
    U: CanLinearInterpolate<V, U>,
    V: Add<Output = V>,
    f32: Mul<V, Output = V>,
    DMatrix<U>: IntoImage,
{
    // Recover motion parameters in the frame of the full image from the one in the cropped frame.
    let motion_vec = match args.crop {
        None => motion_vec_crop.clone(),
        Some(frame) => recover_original_motion(frame, &motion_vec_crop),
    };

    // All that follows is just to help debugging.

    let out_dir_path = Path::new(&args.out_dir);

    // Visualization of cropped and equalized images.
    eprintln!("Saving cropped + equalized images");
    let cropped_dir = out_dir_path.join("cropped");
    lowrr::utils::save_all_imgs(&cropped_dir, &cropped_eq_imgs);

    // Visualization of registered cropped images.
    eprintln!("Saving registered cropped images");
    let registered_cropped_imgs: Vec<DMatrix<T>> =
        registration::reproject::<T, f32, T>(&cropped_eq_imgs, &motion_vec_crop);
    let cropped_aligned_dir = &out_dir_path.join("cropped_aligned");
    lowrr::utils::save_all_imgs(&cropped_aligned_dir, &registered_cropped_imgs);

    // Reproject (interpolation + extrapolation) images according to that motion.
    // Write the registered images to the output directory.
    eprintln!("Saving registered images");
    let registered_imgs = registration::reproject::<U, V, U>(original_imgs, &motion_vec);
    lowrr::utils::save_all_imgs(&out_dir_path, registered_imgs.as_slice());

    Ok(motion_vec)
}

enum Dataset {
    RawImages(Vec<DMatrix<u16>>),
    GrayImages(Vec<DMatrix<u8>>),
    RgbImages(Vec<DMatrix<(u8, u8, u8)>>),
    RgbImagesU16(Vec<DMatrix<(u16, u16, u16)>>),
}

/// Load all images into memory.
fn load_dataset<P: AsRef<Path>>(paths: &[P]) -> anyhow::Result<(Dataset, (usize, usize))> {
    eprintln!("Images to be processed:");
    let images_types: Vec<_> = paths
        .iter()
        .map(|path| {
            eprintln!("    {:?}", path.as_ref());
            match path
                .as_ref()
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .as_deref()
            {
                Some("nef") => "raw",
                Some("png") => "image",
                Some("jpg") => "image",
                Some("jpeg") => "image",
                Some(ext) => panic!("Unrecognized extension: {}", ext),
                None => panic!("Hum no extension?"),
            }
        })
        .collect();

    if images_types.is_empty() {
        anyhow::bail!("There is no such image. Use --help to know how to use this tool.")
    } else if images_types.iter().all(|&t| t == "raw") {
        unimplemented!("imread raw")
    } else if images_types.iter().all(|&t| t == "image") {
        // Open the first image to figure out the image type.
        match image::open(&paths[0])? {
            DynamicImage::ImageRgb8(rgb_img_0) => {
                let (imgs, (height, width)) =
                    load_all(DynamicImage::ImageRgb8(rgb_img_0), &paths[1..]);
                Ok((Dataset::RgbImages(imgs), (width, height)))
            }
            DynamicImage::ImageRgb16(rgb_img_0) => {
                let (imgs, (height, width)) =
                    load_all(DynamicImage::ImageRgb16(rgb_img_0), &paths[1..]);
                Ok((Dataset::RgbImagesU16(imgs), (width, height)))
            }
            _ => anyhow::bail!("Unknow image type"),
        }
    } else {
        panic!("There is a mix of image types")
    }
}

fn load_all<P: AsRef<Path>, Pixel, T: Scalar>(
    first_img: DynamicImage,
    other_paths: &[P],
) -> (Vec<DMatrix<T>>, (usize, usize))
where
    DynamicImage: IntoDMatrix<Pixel, T>,
{
    let img_count = 1 + other_paths.len();
    eprintln!("Loading {} images ...", img_count);
    let pb = indicatif::ProgressBar::new(img_count as u64);
    let mut imgs = Vec::with_capacity(img_count);
    let img_mat = first_img.into_dmatrix();
    let shape = img_mat.shape();
    imgs.push(img_mat);
    pb.inc(1);
    for rgb_img in other_paths.iter().map(|p| image::open(p).unwrap()) {
        imgs.push(rgb_img.into_dmatrix());
        pb.inc(1);
    }
    pb.finish();
    (imgs, shape)
}
