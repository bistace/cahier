#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_in_pty() {
        // simple echo
        let (output, exit_code) = execute_in_pty("echo 'hello world'").expect("failed to execute");
        assert!(output.contains("hello world"));
        assert_eq!(exit_code, Some(0));
    }

    #[test]
    fn test_execute_failure() {
        // command not found
        // bash -c "nonexistent" returns 127
        let (_output, exit_code) = execute_in_pty("nonexistent_command_123").expect("failed to execute");
        assert_ne!(exit_code, Some(0));
    }
}

