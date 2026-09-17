//! Output helpers: human-mode formatting.

pub fn kv(key: &str, value: impl std::fmt::Display) {
    println!("{key:<18} {value}");
}

pub fn header(title: &str) {
    println!("{title}");
    println!("{}", "─".repeat(title.len().max(4)));
}
