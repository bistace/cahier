package executor

import (
	"bytes"
	"os/exec"
	"strings"
)

type Result struct {
	Output   string
	ExitCode int
	Error    error
}

func ExecuteCommand(command string) Result {
	cmd := exec.Command("bash", "-c", command)

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()

	output := stdout.String()
	if stderr.String() != "" {
		if output != "" {
			output += "\n"
		}
		output += stderr.String()
	}

	output = strings.TrimRight(output, "\n")

	exitCode := 0
	if err != nil {
		if exitError, ok := err.(*exec.ExitError); ok {
			exitCode = exitError.ExitCode()
		} else {
			exitCode = -1
		}
	}

	return Result{
		Output:   output,
		ExitCode: exitCode,
		Error:    err,
	}
}
