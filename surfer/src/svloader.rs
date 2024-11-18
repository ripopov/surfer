use crate::info;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead};

pub fn load_enumerates_from_sv_file(
    filename: &str,
) -> Result<HashMap<String, HashMap<String, String>>, std::io::Error> {
    info!("Loading enumerates from file: {}", filename);

    let file = File::open(filename)?;

    let reader = io::BufReader::new(file);
    let mut enum_data: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_enum_name = String::new();
    let mut states = HashMap::new();

    for line in reader.lines() {
        let line = line?;

        if line.trim().is_empty() || line.trim().starts_with("//") {
            continue;
        }

        if let Some((enum_name, _state_lines)) = parse_enum_start(&line) {
            if !current_enum_name.is_empty() {
                enum_data.insert(current_enum_name.clone(), states.clone());
            }

            current_enum_name = enum_name;
            states.clear();
        }

        if let Some((state_name, state_value)) = parse_state(&line) {
            states.insert(state_name, state_value);
        }

        if let Some(wave_names) = parse_end(&line) {
            if !wave_names.is_empty() {
                for name in &wave_names {
                    enum_data.insert(name.clone(), states.clone());
                }
            } else {
                enum_data.insert(current_enum_name.clone(), states.clone());
            }

            current_enum_name.clear();
            states.clear();
        }
    }

    if !current_enum_name.is_empty() {
        enum_data.insert(current_enum_name.clone(), states.clone());
    }

    Ok(enum_data)
}

// Helper function to detect the start of an enum and capture its name
fn parse_enum_start(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();

    if line.starts_with("enum") && line.contains("{") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let enum_name = parts[1].to_string();
            return Some((enum_name, Vec::new()));
        }
    }
    None
}

// Will be fine for now (in current case) but update to support more complex state definitions later
fn parse_state(line: &str) -> Option<(String, String)> {
    let line = line.trim();

    if let Some(comma_pos) = line.find(',') {
        let state_name = line[..comma_pos].trim().to_string();

        if let Some(comment_pos) = line.find("//") {
            let state_value = line[comment_pos + 2..].trim().to_string();
            return Some((state_value, state_name));
        }
    } else {
        // assume it's the last state
        let state_name = line.trim().to_string();
        if let Some(comment_pos) = line.find("//") {
            let state_value = line[comment_pos + 2..].trim().to_string();
            return Some((state_value, state_name));
        }
    }
    None
}

fn parse_end(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if let Some(curly_pos) = line.find('}') {
        if let Some(semi_pos) = line.find(';') {
            if curly_pos + 1 < semi_pos {
                let r_string = line[(curly_pos + 1)..semi_pos].trim().to_string();

                let names: Vec<String> =
                    r_string.split(',').map(|s| s.trim().to_string()).collect();

                return Some(names);
            }
        }
    }
    None
}
