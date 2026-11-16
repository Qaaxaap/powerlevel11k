//! Build-time inputs for `static_loader!`.
//!
//! `static_loader!` reads `locales/` while the macro expands. Cargo tracks the files the generated
//! code already includes, so editing an existing catalogue rebuilds the crate. A newly added
//! `locales/<lang>.ftl` is not tracked, so adding one keeps the old embedded set until the macro
//! runs again. The directory input makes that rebuild happen, which is what `CONTRIBUTING.md`
//! step 5 promises.

fn main() {
    println!("cargo:rerun-if-changed=locales");
}
