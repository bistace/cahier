package history

import (
	"cahier/store"
	"fmt"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
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
			Width(4).
			Align(lipgloss.Right).
			MarginRight(1)

	// Base style for selected cell number (color will be dynamic)
	selectedCellNumberBaseStyle = lipgloss.NewStyle().
					Width(4).
					Align(lipgloss.Right).
					MarginRight(1).
					Bold(true)

	// Content container (width will be set dynamically)
	cellContentStyle = lipgloss.NewStyle()

	// Empty state style
	emptyStateStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("#B19CD9")).
			Italic(true).
			Padding(2, 4)
)

type Model struct {
	commands      []store.Command
	selected      int
	colorIndex    int
	lastUpdate    time.Time
	terminalWidth int
	viewport      viewport.Model
	ready         bool
}

func NewModel(commands []store.Command) Model {
	vp := viewport.New(80, 10)
	vp.Style = lipgloss.NewStyle()

	return Model{
		commands:      commands,
		selected:      -1,
		colorIndex:    0,
		lastUpdate:    time.Now(),
		terminalWidth: 80, // Default width
		viewport:      vp,
		ready:         false,
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

	if !m.ready {
		return "Initializing..."
	}

	return m.viewport.View()
}

func (m *Model) renderContent() string {
	var cells []string
	for i, cmd := range m.commands {
		// Create cell number without brackets
		cellNum := fmt.Sprintf("%d:", i+1)

		// Choose styles based on selection
		var cellStyle, numberStyle lipgloss.Style
		var currentContentWidth int
		if i == m.selected {
			// Create animated rainbow border and number for selected cell
			rainbowColor := rainbowColors[m.colorIndex]
			cellStyle = selectedCellBaseStyle.BorderForeground(lipgloss.Color(rainbowColor))
			numberStyle = selectedCellNumberBaseStyle.Foreground(lipgloss.Color(rainbowColor))
			// Selected cell has more padding
			currentContentWidth = m.terminalWidth - 4 - 1 - 6 - 2
		} else {
			cellStyle = normalCellStyle
			numberStyle = cellNumberStyle
			currentContentWidth = m.terminalWidth - 4 - 1 - 4 - 2
		}

		if currentContentWidth < 20 {
			currentContentWidth = 20 // Minimum width
		}

		// Render cell number and content with dynamic width
		cellNumber := numberStyle.Render(cellNum)
		cellContent := cellContentStyle.Width(currentContentWidth).Render(cmd.Command)

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
		m.updateViewport()
		m.ensureSelectedVisible()
	}
}

func (m *Model) ensureSelectedVisible() {
	if m.selected < 0 || !m.ready {
		return
	}

	// Calculate approximate line positions for each command
	// Each command takes about 3-4 lines (with padding and borders)
	lineHeight := 4
	selectedLine := m.selected * lineHeight

	// Check if selected item is above viewport
	if selectedLine < m.viewport.YOffset {
		m.viewport.SetYOffset(selectedLine)
	}

	// Check if selected item is below viewport
	viewportBottom := m.viewport.YOffset + m.viewport.Height
	if selectedLine+lineHeight > viewportBottom {
		m.viewport.SetYOffset(selectedLine + lineHeight - m.viewport.Height)
	}
}

func (m *Model) ScrollToBottom() {
	if !m.ready {
		return
	}
	m.viewport.GotoBottom()
}

func (m *Model) updateViewport() {
	content := m.renderContent()
	m.viewport.SetContent(content)
}

func (m *Model) SetCommands(commands []store.Command) {
	m.commands = commands
	if m.selected >= len(commands) {
		m.selected = len(commands) - 1
	}
	m.updateViewport()
	m.ScrollToBottom()
}

func (m *Model) SetWidth(width int) {
	m.terminalWidth = width
	m.viewport.Width = width
	m.updateViewport()
}

func (m *Model) SetHeight(height int, isEditMode bool) {
	// Calculate available height:
	// - App header: 3 lines (title + 2 newlines)
	// - Footer: 1 line
	// - Bottom margin: 2 lines
	reservedLines := 6

	// Reserve additional space when in edit mode for the textarea
	if isEditMode {
		// Textarea takes about 5-6 lines with borders and padding
		reservedLines += 6
	}

	adjustedHeight := max(height-reservedLines, 5)
	m.viewport.Height = adjustedHeight
	wasReady := m.ready
	m.ready = true
	m.updateViewport()

	// Scroll to bottom on initial load
	if !wasReady {
		m.ScrollToBottom()
	} else if isEditMode {
		// When entering edit mode, always scroll to show the bottom commands
		// This ensures the last commands are visible above the textarea
		m.ScrollToBottom()
	} else {
		m.ensureSelectedVisible()
	}
}

func (m Model) Update(msg tea.Msg) (Model, tea.Cmd) {
	var cmd tea.Cmd
	m.viewport, cmd = m.viewport.Update(msg)
	return m, cmd
}
