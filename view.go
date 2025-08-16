package main

import (
	"fmt"
	"github.com/charmbracelet/lipgloss"
)

var (
	appNameStyle = lipgloss.NewStyle().Background(lipgloss.Color("99")).Padding(0, 1)
	faintStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("255")).Faint(true)

	// Base style for textarea container
	textareaStyle = lipgloss.NewStyle().
			Padding(1, 1).
			Margin(0, 0, 0, 0).
			BorderStyle(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#E8E8E8"))

	// Style for the label
	textareaLabelStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#B19CD9")).
				Width(5).
				Align(lipgloss.Right).
				MarginRight(1)
)

func (m Model) View() string {
	s := appNameStyle.Render("Cahier") + "\n\n"

	s += m.cmdsHistory.View() + "\n\n"

	if m.currentMode == EditMode {
		var label string
		if m.currentIdx == -1 {
			label = textareaLabelStyle.Render("New:")
		} else {
			label = textareaLabelStyle.Render(fmt.Sprintf("%d:", m.currentIdx+1))
		}

		// Render the textarea with the styled container
		textareaContent := textareaStyle.Render(m.textarea.View())

		// Combine label with textarea
		s += lipgloss.JoinHorizontal(
			lipgloss.Center,
			label,
			textareaContent,
		) + "\n\n"
	}

	switch m.currentMode {
	case ViewMode:
		s += faintStyle.Render("n: New cell - enter: Edit selected command - ctrl+d: Quit")
	case EditMode:
		s += faintStyle.Render("ctrl+r: Run - escape: Cancel - ctrl+d: Quit")
	}

	return s
}
