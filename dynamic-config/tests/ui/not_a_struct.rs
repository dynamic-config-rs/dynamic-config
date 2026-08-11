//! The attribute names what it needs when put on the wrong item — and the
//! error arrives alone, not buried under a cascade from the vanished type.

#[dynamic_config::dynamic_config]
enum NotAStruct {
    Variant,
}

fn main() {}
