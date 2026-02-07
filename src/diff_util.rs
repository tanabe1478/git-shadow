use colored::Colorize;

/// Generate unified diff output between old and new text
pub fn unified_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut output = String::new();

    output.push_str(&format!("--- {}\n", old_label));
    output.push_str(&format!("+++ {}\n", new_label));

    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&hunk.to_string());
    }

    output
}

/// Print unified diff with colors to stdout
pub fn print_colored_diff(old: &str, new: &str, old_label: &str, new_label: &str) {
    let diff = similar::TextDiff::from_lines(old, new);

    println!("{}", format!("--- {}", old_label).red());
    println!("{}", format!("+++ {}", new_label).green());

    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        for line in hunk.to_string().lines() {
            if line.starts_with("@@") {
                println!("{}", line.cyan());
            } else if line.starts_with('+') {
                println!("{}", line.green());
            } else if line.starts_with('-') {
                println!("{}", line.red());
            } else {
                println!("{}", line);
            }
        }
    }
}

/// Print full file content as a "new file" diff
pub fn print_new_file_diff(content: &str, file_path: &str) {
    println!("{}", "--- /dev/null".red());
    println!("{}", format!("+++ {}", file_path).green());
    println!(
        "{}",
        format!("@@ -0,0 +1,{} @@", content.lines().count()).cyan()
    );
    for line in content.lines() {
        println!("{}", format!("+{}", line).green());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_diff_no_change() {
        let result = unified_diff("hello\n", "hello\n", "a/file", "b/file");
        assert!(result.contains("--- a/file"));
        assert!(result.contains("+++ b/file"));
        // No hunks for identical content
        assert!(!result.contains("@@"));
    }

    #[test]
    fn test_unified_diff_added_lines() {
        let result = unified_diff("line1\n", "line1\nline2\n", "a/file", "b/file");
        assert!(result.contains("+line2"));
        assert!(result.contains("@@"));
    }

    #[test]
    fn test_unified_diff_removed_lines() {
        let result = unified_diff("line1\nline2\n", "line1\n", "a/file", "b/file");
        assert!(result.contains("-line2"));
    }

    #[test]
    fn test_unified_diff_mixed() {
        let result = unified_diff("old\n", "new\n", "a/file", "b/file");
        assert!(result.contains("-old"));
        assert!(result.contains("+new"));
    }

    #[test]
    fn test_unified_diff_empty_to_content() {
        let result = unified_diff("", "new content\n", "a/file", "b/file");
        assert!(result.contains("+new content"));
    }
}
