package main

import (
	"github.com/charmbracelet/lipgloss"
)

var (
	appNameStyle    = lipgloss.NewStyle().Background(lipgloss.Color("99")).Padding(0, 1)
	faintStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color("255")).Faint(true)
	enumeratorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("99")).MarginRight(1)
)

func (m Model) View() string {
	s := appNameStyle.Render("Cahier") + "\n\n"

	// for i, c := range m.cmds {
	// 	prefix := " "
	// 	if i == m.currentIdx {
	// 		prefix = ">"
	// 	}
	// 	s += enumeratorStyle.Render(prefix) + c.Command + "\n\n"

	// 	if m.currentMode == EditMode && i == m.currentIdx {
	// 		s += m.textarea.View() + "\n\n"
	// 	}
	// }
	s += m.cmdsHistory.View()

	if m.currentMode == EditMode && m.currentIdx == -1 {
		s += m.textarea.View() + "\n\n"
	}

	switch m.currentMode {
	case ViewMode:
		s += faintStyle.Render("n: new cell - ctrl+d: Quit")
	case EditMode:
		s += faintStyle.Render("escape: Cancel - ctrl+d: Quit")
	}

	return s
}
