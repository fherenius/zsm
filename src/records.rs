//! Encoding shared records without putting paths or names into shell syntax.

pub fn encode(value: &str) -> String {
    if value.is_empty() {
        return "-".into();
    }
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn decode(encoded: &str) -> Option<String> {
    if encoded == "-" {
        return Some(String::new());
    }
    if encoded.is_empty() || encoded.len() % 2 != 0 || !encoded.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<_>> = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}
