// `embed_migrations!` reads `migrations/` at compile time, so rebuild when a
// migration is added or changed.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
