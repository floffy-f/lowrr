use image::GenericImageView;
use lowrr::interop;
use lowrr::registration;
mod unused;

use glob::glob;
use nalgebra::DMatrix;
use std::path::{Path, PathBuf};
use std::str::FromStr;

// Default values for some of the program arguments.
const DEFAULT_OUT_DIR: &str = "out";
const DEFAULT_CROP: Crop = Crop::NoCrop;
const DEFAULT_LEVELS: usize = 4;
const DEFAULT_LAMBDA: f32 = 1.5;
const DEFAULT_RHO: f32 = 0.1;
const DEFAULT_THRESHOLD: f32 = 1e-3;
const DEFAULT_MAX_ITERATIONS: usize = 20;
const DEFAULT_IMAGE_MAX: f32 = 255.0;

/// Entry point of the program.
fn main() {
    parse_args()
        .and_then(run)
        .unwrap_or_else(|err| eprintln!("Error: {:?}", err));
}

fn display_help() {
    eprintln!(
        r#"
lowrr

Low-rank registration of slightly unaligned images for photometric stereo.
Some algorithm info is output to stderr while running.
You can ignore them by redirecting stderr to /dev/null.
The final motion vector is written to stdout,
you can redirect it to a file with the usual pipes.

USAGE:
    lowrr [FLAGS] IMAGE_FILES
    For example:
        lowrr --trace *.png
        lowrr *.jpg 2> /dev/null
        lowrr *.png > result.txt

FLAGS:
    --help                 # Print this message and exit
    --version              # Print version and exit
    --out-dir dir/         # Output directory to save registered images (default: {})
    --trace                # Print more debug output to stderr while running
    --crop x1,y1,x2,y2     # Crop image into a restricted working area (use no space between coordinates)
    --no-image-correction  # Avoid image correction
    --levels int           # Number of levels for the multi-resolution approach (default: {})
    --lambda float         # Weight of the L1 term (high means no correction) (default: {})
    --rho float            # Lagrangian penalty (default: {})
    --threshold float      # Stop when relative diff between two estimate of corrected image falls below this (default: {})
    --max-iterations int   # Maximum number of iterations (default: {})
    --image-max float      # Maximum possible value of the images for scaling (default: {})
"#,
        DEFAULT_OUT_DIR,
        DEFAULT_LEVELS,
        DEFAULT_LAMBDA,
        DEFAULT_RHO,
        DEFAULT_THRESHOLD,
        DEFAULT_MAX_ITERATIONS,
        DEFAULT_IMAGE_MAX,
    )
}

#[derive(Debug)]
/// Type holding command line arguments.
struct Args {
    config: registration::Config,
    help: bool,
    version: bool,
    out_dir: String,
    images_paths: Vec<PathBuf>,
    crop: Crop,
}

/// Function parsing the command line arguments and returning an Args object or an error.
fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = pico_args::Arguments::from_env();

    // Retrieve command line arguments.
    let help = args.contains(["-h", "--help"]);
    let version = args.contains(["-v", "--version"]);
    let do_image_correction = !args.contains("--no-image-correction");
    let trace = args.contains("--trace");
    let crop = args.opt_value_from_str("--crop")?.unwrap_or(DEFAULT_CROP);
    let lambda = args
        .opt_value_from_str("--lambda")?
        .unwrap_or(DEFAULT_LAMBDA);
    let rho = args.opt_value_from_str("--rho")?.unwrap_or(DEFAULT_RHO);
    let threshold = args
        .opt_value_from_str("--threshold")?
        .unwrap_or(DEFAULT_THRESHOLD);
    let max_iterations = args
        .opt_value_from_str("--max-iterations")?
        .unwrap_or(DEFAULT_MAX_ITERATIONS);
    let levels = args
        .opt_value_from_str("--levels")?
        .unwrap_or(DEFAULT_LEVELS);
    let image_max = args
        .opt_value_from_str("--image-max")?
        .unwrap_or(DEFAULT_IMAGE_MAX);
    let out_dir = args
        .opt_value_from_str("--out-dir")?
        .unwrap_or(DEFAULT_OUT_DIR.into());

    // Verify that images paths are correct.
    let free_args = args.free()?;
    let images_paths = absolute_file_paths(&free_args)?;

    // Return Args struct.
    Ok(Args {
        config: registration::Config {
            do_image_correction,
            trace,
            lambda,
            rho,
            threshold,
            max_iterations,
            levels,
            image_max,
        },
        help,
        version,
        out_dir,
        images_paths,
        crop,
    })
}

/// Retrieve the absolute paths of all files matching the arguments.
fn absolute_file_paths(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut abs_paths = Vec::new();
    for path_glob in args {
        let mut paths = paths_from_glob(path_glob)?;
        abs_paths.append(&mut paths);
    }
    abs_paths
        .iter()
        .map(|p| p.canonicalize().map_err(|e| e.into()))
        .collect()
}

/// Retrieve the paths of files matchin the glob pattern.
fn paths_from_glob(p: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let paths = glob(p)?;
    Ok(paths.into_iter().filter_map(|x| x.ok()).collect())
}

#[derive(Debug)]
enum Crop {
    NoCrop,
    Area(usize, usize, usize, usize),
}

impl FromStr for Crop {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = s.splitn(4, ',').collect();
        if parts.len() != 4 {
            panic!(
                "--crop argument must be of the shape x1,y1,x2,y2 with no space between elements"
            );
        }
        let x1 = parts[0].parse()?;
        let y1 = parts[1].parse()?;
        let x2 = parts[2].parse()?;
        let y2 = parts[3].parse()?;
        Ok(Crop::Area(x1, y1, x2, y2))
    }
}

/// Start actual program with command line arguments successfully parsed.
fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // Check if the --help or --version flags are present.
    if args.help {
        display_help();
        std::process::exit(0);
    } else if args.version {
        println!("{}", std::env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    // Get the path of output directory.
    let out_dir_path = PathBuf::from(args.out_dir);

    // Load the dataset in memory.
    let (dataset, _) = load_dataset(args.crop, &args.images_paths)?;

    // Use the algorithm corresponding to the type of data.
    match dataset {
        Dataset::GrayImages(imgs) => {
            // Compute the motion of each image for registration.
            let motion_vec = registration::gray_images(args.config, imgs.clone())?;

            // Reproject (interpolation + extrapolation) images according to that motion.
            let registered_imgs = registration::reproject_u8(&imgs, &motion_vec);

            // Write the registered images to the output directory.
            std::fs::create_dir_all(&out_dir_path)
                .expect(&format!("Could not create output dir: {:?}", &out_dir_path));
            registered_imgs.iter().enumerate().for_each(|(i, img)| {
                interop::image_from_matrix(img)
                    .save(&out_dir_path.join(format!("{}.png", i)))
                    .expect("Error saving image");
            });

            // Write motion_vec to stdout.
            for v in motion_vec.iter() {
                println!("{} {}", v.x, v.y)
            }
            Ok(())
        }
        Dataset::RgbImages { red, green, blue } => unimplemented!(),
        Dataset::RawImages(imgs) => unimplemented!(),
    }
}

enum Dataset {
    RawImages(Vec<DMatrix<u16>>),
    GrayImages(Vec<DMatrix<u8>>),
    RgbImages {
        red: Vec<DMatrix<u8>>,
        green: Vec<DMatrix<u8>>,
        blue: Vec<DMatrix<u8>>,
    },
}

/// Load all images into memory.
fn load_dataset<P: AsRef<Path>>(
    crop: Crop,
    paths: &[P],
) -> Result<(Dataset, (usize, usize)), Box<dyn std::error::Error>> {
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
        Err("There is no such image. Use --help to know how to use this tool.".into())
    } else if images_types.iter().all(|&t| t == "raw") {
        unimplemented!("imread raw")
    } else if images_types.iter().all(|&t| t == "image") {
        let images: Vec<DMatrix<u8>> = paths
            .iter()
            .map(|path| image::open(path).unwrap())
            .map(|mut i| match crop {
                Crop::NoCrop => i,
                Crop::Area(x1, y1, x2, y2) => {
                    let x1 = x1 as u32;
                    let y1 = y1 as u32;
                    let x2 = x2 as u32;
                    let y2 = y2 as u32;
                    assert!(x1 < i.width(), "Error: x1 >= image width");
                    assert!(x2 < i.width(), "Error: x2 >= image width");
                    assert!(y1 < i.height(), "Error: y1 >= image height");
                    assert!(y2 < i.height(), "Error: y2 >= image height");
                    assert!(x1 <= x2, "Error: x2 must be greater than x1");
                    assert!(y1 <= y2, "Error: y2 must be greater than y1");
                    i.crop(x1, y1, x2 - x1, y2 - y1)
                }
            })
            // Temporary convert color to gray.
            // .map(|i| i.into_luma())
            // .map(interop::matrix_from_image)
            .map(|i| i.into_rgb8())
            .map(interop::matrix_from_rgb_image)
            // Temporary only keep one channel.
            .map(|m| m.map(|(_red, green, _blue)| green))
            .collect();
        let (height, width) = images[0].shape();
        Ok((Dataset::GrayImages(images), (width, height)))
    } else {
        panic!("There is a mix of image types")
    }
}
