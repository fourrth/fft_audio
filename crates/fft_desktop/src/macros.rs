//! Reusable clap definition helper for feature-gated product arguments.
//!
//! Product features select which adapters and CLI arguments exist. This macro
//! keeps that mapping in one place: every `features(...)` group becomes its own
//! clap `Args` module and is compiled only when all listed cargo features are
//! enabled. `common` is always present. Adding an adapter feature means adding
//! one group, not threading `#[cfg]` through the parser or help surface.
//!
//! Usage:
//!
//! ```ignore
//! feature_cli_args! {
//!     #[command(name = "fft_desktop", version)]
//!     common {
//!         /// Runtime worker threads.
//!         #[arg(long, default_value_t = 1)]
//!         workers: usize,
//!     }
//!     features(http: "http") {
//!         /// Serve the WebUI.
//!         #[arg(long)]
//!         serve: bool,
//!     }
//!     features(python: "python") {
//!         /// Local Python script to run.
//!         #[arg(long = "python-script")]
//!         python_scripts: Vec<std::path::PathBuf>,
//!     }
//! }
//! ```
//!
//! The generated `CliArgs` derives `clap::Parser` and flattens every group.
//! Groups are modules named after their identifier (`http`, `python`, ...), so
//! conversions read `args.http.serve` or `args.python.python_scripts`.
#[macro_export]
macro_rules! feature_cli_args {
    (
        $(#[$command:meta])*
        common {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $field_ty:ty
            ),* $(,)?
        }
        $(
            features($group:ident : $($feature:literal),+ $(,)?) {
                $(
                    $(#[$group_meta:meta])*
                    $group_field:ident : $group_ty:ty
                ),* $(,)?
            }
        )*
    ) => {
        pub mod common {
            use super::*;

            #[derive(::clap::Args, Debug)]
            #[group(id = "common")]
            pub struct Args {
                $(
                    $(#[$field_meta])*
                    pub $field: $field_ty,
                )*
            }
        }

        $(
            #[cfg(all($(feature = $feature),+))]
            pub mod $group {
                use super::*;

                #[derive(::clap::Args, Debug)]
                #[group(id = stringify!($group))]
                pub struct Args {
                    $(
                        $(#[$group_meta])*
                        pub $group_field: $group_ty,
                    )*
                }
            }
        )*

        #[derive(::clap::Parser, Debug)]
        $(#[$command])*
        pub struct CliArgs {
            #[command(flatten)]
            pub common: common::Args,
            $(
                #[cfg(all($(feature = $feature),+))]
                #[command(flatten)]
                pub $group: $group::Args,
            )*
        }
    };
}
