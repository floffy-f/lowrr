# low-rank-registration

Low-rank registration of slightly misaligned images for photometric stereo.
This repository holds both the `lowrr` library, and a `lowrr` command-line executable.

> Matthieu Pizenberg, Yvain Quéau, Abderrahim Elmoataz,
> "Low-rank registration of images captured under unknown, varying lighting".
> International Conference on Scale Space and Variational Methods in Computer Vision (SSVM).
> 2021.

The algorithm presented here takes advantage of the fact that well aligned sets of images
should form a low rank matrix.
We thus minimize the nuclear norm of that matrix (sum of singular values),
which is the convex relaxation of its rank.

This algorithm gives convincing results in the context of photometric stereo images,
which is where we have evaluated it,
but it should also work reliably in other situations where minimizing the rank makes sense.
Some additional experiments show interesting results with multimodal images for example.

![Alignment of photometric stereo images improves the 3D reconstruction][handheld]

The previous figure showcases the improvement of both the 3D reconstruction,
and the recovered albedo after an alignment of handheld photos of the
Bayeux Tapestry.

[handheld]: https://mpizenberg.github.io/resources/lowrr/handheld.jpg

## Acknowledgements

This work was supported by the RIN project "Guide Muséal",
and by the ANR grant "Inclusive Museum Guide" (ANR-20-CE38-0007).
The authors would like to thank C. Berthelot at the Bayeux Tapestry Museum
for supervising the image acquisition campaign of the Bayeux Tapestry.

## Installation

To install the `lowrr` command-line program,
simply download the executable for your platform on the latest release.
TODO: make a GitHub release.
Then simply extract and put it in a directory listed in your `PATH` environment variable.
This way, you will be able to call `lowrr` from anywhere.

## Usage

The simplest way to use `lowrr` is to call it with a glob pattern
for the images you want to align, for example:

```sh
lowrr img/*.png
```

By default, this will compute the registration and output to stdout
the affine parameters of each image transformation as specified
in our research paper.

If you also want to apply the transformation and save the registered images,
you can add the `--save-imgs` command line argument.

```sh
# Apply the transformation and save the registered images
lowrr --save-imgs img/*.png
```

Usually, the algorithm can estimate the aligning transformation without working
on the whole image, but just a cropped area of the image to make things faster.
You can specify that working frame with the command line arguments
`--crop <left> <top> <right> <bottom>` where the border coordinates of that frame
are specified after the `--crop` argument (top-left corner is 0,0).
In that case, I'd suggest to also add the `--save-crop` argument
to be able to visualize the cropped area and its registration.

```sh
# Work on a reduced 500x300 cropped area and visualize its registration
lowrr --crop 0 0 500 300 --save-crop img/*.png
```

You can also customize all the algorithm parameters.
For more info, have a look at the program help.

```sh
# Display the program help for more info
lowrr --help
```

## Lib documentation

In addition to the `lowrr` executable, that is compiled from `src/main.rs`,
we also provide the code in the form of a library,
so that it can easily be re-used for other Rust applications.
The library code is fully documented, and the documentation is automatically
generated and made available at
https://matthieu.pizenberg.pages.unicaen.fr/low-rank-registration

## Unfamiliar with Rust?

If you want to read the source code but are not very familiar
with the Rust language, here are few syntax explanations.

Basically, if you know how to read C/C++ code, the structure of Rust
code should be pretty familiar.
For example, it uses curly braces to delimit code blocks
and the parts between brackets `<T>` are type parameters,
like templates in C++.

Here are code examples of some patterns and syntax that may be new though.

```rust
// Pattern 1: closures
let square = |x| x * x;
square(3) // -> 9

// Pattern 2: iterators
xCollection.iter().map(|x| f(x)).collect();

// Pattern 3: zipping iterators
xCollection.iter()
    .zip(yCollection.iter())
    .map(|(x,y)| f(x,y)).collect();

// Pattern 4: for loops on iterators
for x in xCollection.iter() {
    do_something_with(x)
}

// Pattern 5: crashing on potential errors
result.unwrap();
// or
result.expect("crash with an error message");
```

The first pattern is the usage of "closures",
a.k.a. "anonymous functions", a.k.a. "lambda functions".
The part between the bars `|x|` are the arguments.
The part after the bars `x * x` is the returned value.
Closures are useful to use instead of defining properly
named functions in some parts of the program.

The second pattern (`.iter().map(...)`) is basically saying that
we are iterating over a collection of things and we apply
the same function `f` to all those elements of the collection.
The `collect()` at the end is more or less saying that we are done
modifying it in this iterator, and we can regenerate a new
data structure that will contain the result of those modifications.

The third pattern consists in using `iterator1.zip(iterator2)`.
It is just to bring together two iterators and apply a function
to both elements at the same time.

Pattern 4 is another way of iterating, similar to pattern 1.
Depending on the situation, using loops or mapping a function will be more appropiate.

Finally, the usage of `unwrap()` or `expect(...)` is just to say
to the compiler that I know it is safe to extract a potentially failing value
even though it may result in an error.
In the case of an error, this will crash the program,
and print the message inside the `expect(...)`.

## Code contribution

To compile the source code yourself, you just need to install [Rust][rust],
and then run the command `cargo build --release`.
Cargo is Rust build tool, it will automatically download dependencies
and compile all the code.
The resulting binary will be located in `target/release/`.
The first compilation may take a little while, but then will be pretty fast.

[rust]: https://www.rust-lang.org/tools/install
