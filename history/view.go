package history

import (
	"cahier/store"
	"fmt"
	"github.com/charmbracelet/lipgloss"
	"time"
)

var (
	// Rainbow colors for animated border (pastel rainbow with smooth transitions)
	rainbowColors = []string{
		"#FFB3BA", // Pastel pink
		"#FFC7B3", // Pink-orange transition
		"#FFDAB3", // Pastel orange
		"#FFEDB3", // Orange-yellow transition
		"#FFFFB3", // Pastel yellow
		"#D7FFB3", // Yellow-green transition
		"#BAFFB3", // Pastel green
		"#B3FFD7", // Green-cyan transition
		"#B3FFFF", // Pastel cyan
		"#B3E5FF", // Cyan-blue transition
		"#B3CCFF", // Light blue
		"#B3BAFF", // Pastel blue
		"#C7B3FF", // Blue-purple transition 1
		"#D3B3FF", // Blue-purple transition 2
		"#E0B3FF", // Pastel purple
		"#EDB3FF", // Purple-magenta transition
		"#FFB3F0", // Pastel magenta
		"#FFB3D7", // Magenta-pink transition
	}

	// Cell container styles
	normalCellStyle = lipgloss.NewStyle().
			Padding(1, 1).
			Margin(0, 0, 0, 0).
			BorderStyle(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#E8E8E8")) // Light gray border

	// Base style for selected cell (border color will be dynamic)
	selectedCellBaseStyle = lipgloss.NewStyle().
				Padding(1, 2).
				Margin(0, 0, 0, 0).
				BorderStyle(lipgloss.ThickBorder()).
				Bold(true)

	// Cell number styles
	cellNumberStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#B19CD9")). // Muted purple
			Width(8).
			Align(lipgloss.Right).
			MarginRight(1)

	// Base style for selected cell number (color will be dynamic)
	selectedCellNumberBaseStyle = lipgloss.NewStyle().
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
	commands   []store.Command
	selected   int
	colorIndex int
	lastUpdate time.Time
}

func NewModel(commands []store.Command) Model {
	return Model{
		commands:   commands,
		selected:   -1,
		colorIndex: 0,
		lastUpdate: time.Now(),
	}
}

func (m *Model) UpdateAnimation() {
	// Update color index for rainbow animation (cycle every 100ms)
	if time.Since(m.lastUpdate) > 200*time.Millisecond {
		m.colorIndex = (m.colorIndex + 1) % len(rainbowColors)
		m.lastUpdate = time.Now()
	}
}

func (m Model) View() string {
	if len(m.commands) == 0 {
		return emptyStateStyle.Render("📝 No commands yet. Press 'n' to create one.")
	}

	var cells []string
	for i, cmd := range m.commands {
		// Create cell number like "[1]:"
		cellNum := fmt.Sprintf("[%d]:", i+1)

		// Choose styles based on selection
		var cellStyle, numberStyle lipgloss.Style
		if i == m.selected {
			// Create animated rainbow border and number for selected cell
			rainbowColor := rainbowColors[m.colorIndex]
			cellStyle = selectedCellBaseStyle.BorderForeground(lipgloss.Color(rainbowColor))
			numberStyle = selectedCellNumberBaseStyle.Foreground(lipgloss.Color(rainbowColor))
		} else {
			cellStyle = normalCellStyle
			numberStyle = cellNumberStyle
		}

		// Render cell number and content
		cellNumber := numberStyle.Render(cellNum)
		cellContent := cellContentStyle.Render(cmd.Command)

		// Combine cell number with the cell container (center vertically)
		cell := lipgloss.JoinHorizontal(
			lipgloss.Center,
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
