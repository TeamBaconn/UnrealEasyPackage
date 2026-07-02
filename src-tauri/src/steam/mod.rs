//! Steam upload phase (`docs/build-commands.md` §11).
//!
//! Steam distribution runs on **SteamPipe**: the only programmatic upload is
//! `steamcmd +run_app_build <app_build.vdf>`, which pushes an archived build tree to a
//! depot. This module owns the two Steam-specific pieces the pipeline needs:
//!
//! - [`vdf`] - a minimal Valve-KeyValues tree + the `app_build`/`depot_build` generators.
//!   The **committed** VDFs (`<project>/.uep/steam-config/<profile-id>/`) carry the
//!   profile's managed fields *and preserve any custom keys the user adds*; a **resolved**
//!   copy (with `ContentRoot`/`BuildOutput` injected) is materialized into the git-ignored
//!   scratch dir (`.uep/steam-build-output/<profile-id>/`) at run time.
//! - [`login`] - a non-interactive `steamcmd +login <account> +quit` **session check**
//!   ([`login::verify`]). No password is entered in the app: when a build reaches the upload
//!   step, the runner checks the session and, if it's missing, opens steamcmd in its own
//!   console for the user to sign in there (Steam Guard code, or mobile-app confirmation).
//!
//! The steamcmd *command* itself (`+login <account> +run_app_build … +quit`) is built by
//! `unreal::args` alongside the other phase commands, and the run's VDF pre-step + the
//! signed-in/console decision live in the runner (`runner::exec`).
#![allow(dead_code)]

pub mod login;
pub mod vdf;

pub use vdf::run_app_build_vdf_path;
