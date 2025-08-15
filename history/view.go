package history

import (
	"cahier/store"
	"fmt"
	"github.com/charmbracelet/lipgloss"
)

var (
	// Cell container styles
	normalCellStyle = lipgloss.NewStyle().
		// Background(lipgloss.Color("#F0E6FF")). // Light lavender
		// Foreground(lipgloss.Color("#2D2D2D")). // Dark gray text
		Padding(1, 1).
		Margin(0, 0, 0, 0).
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#E8E8E8")) // Light gray border

	selectedCellStyle = lipgloss.NewStyle().
		// Background(lipgloss.Color("#E6D7FF")). // Pastel purple
		// Foreground(lipgloss.Color("#2D2D2D")). // Dark gray text
		Padding(1, 2).
		Margin(0, 0, 1, 0).
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#FFD4B2")). // Peach border
		Bold(true)

	// Cell number styles
	cellNumberStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#B19CD9")). // Muted purple
			Width(8).
			Align(lipgloss.Right).
			MarginRight(1)

	selectedCellNumberStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("#9370DB")). // Slightly brighter purple for selected
				Width(8).
				Align(lipgloss.Right).
				MarginRight(1).
				Bold(true)

	// Content container
	cellContentStyle = lipgloss.NewStyle().
				MaxWidth(80)

	// Empty state style
	emptyStateStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#B19CD9")).
			Italic(true).
			Padding(2, 4)
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
		return emptyStateStyle.Render("📝 No commands yet. Press 'n' to create one.")
	}

	var cells []string
	for i, cmd := range m.commands {
		// Create cell number like "In[1]:"
		cellNum := fmt.Sprintf("In[%d]:", i+1)

		// Choose styles based on selection
		var cellStyle, numberStyle lipgloss.Style
		if i == m.selected {
			cellStyle = selectedCellStyle
			numberStyle = selectedCellNumberStyle
		} else {
			cellStyle = normalCellStyle
			numberStyle = cellNumberStyle
		}

		// Render cell number and content
		cellNumber := numberStyle.Render(cellNum)
		cellContent := cellContentStyle.Render(cmd.Command)

		// Combine cell number with the cell container
		cell := lipgloss.JoinHorizontal(
			lipgloss.Top,
			cellNumber,
			cellStyle.Render(cellContent),
		)

		cells = append(cells, cell)
	}

	// Join all cells vertically
	return lipgloss.JoinVertical(lipgloss.Left, cells...)
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
