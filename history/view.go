package history

import (
	"fmt"
	"cahier/store"
	"github.com/charmbracelet/lipgloss"
)

var (
	selectedStyle = lipgloss.NewStyle().
		Foreground(lipgloss.Color("170")).
		Bold(true)
	normalStyle = lipgloss.NewStyle().
		Foreground(lipgloss.Color("252"))
	cursorStyle = lipgloss.NewStyle().
		Foreground(lipgloss.Color("170")).
		Bold(true)
)

type Model struct {
	commands []store.Command
	selected int
}

func NewModel(commands []store.Command) Model {
	return Model{
		commands: commands,
		selected: -1,
	}
}

func (m Model) View() string {
	if len(m.commands) == 0 {
		return normalStyle.Render("No commands yet. Press 'n' to create one.")
	}

	var s string
	for i, cmd := range m.commands {
		cursor := "  "
		style := normalStyle
		
		if i == m.selected {
			cursor = "> "
			style = selectedStyle
		}
		
		line := fmt.Sprintf("%s%s", cursorStyle.Render(cursor), style.Render(cmd.Command))
		s += line + "\n"
	}
	
	return s
}

func (m *Model) Select(index int) {
	if index >= 0 && index < len(m.commands) {
		m.selected = index
	}
}

func (m *Model) SetCommands(commands []store.Command) {
	m.commands = commands
	if m.selected >= len(commands) {
		m.selected = len(commands) - 1
	}
}