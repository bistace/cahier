package main

import (
	"log"
	"strings"
	"time"

	"cahier/history"
	"cahier/store"

	"github.com/charmbracelet/bubbles/textarea"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type Status int

const (
	ViewMode       Status = iota
	EditMode              // For inline editing of existing commands
	NewCommandMode        // For creating new commands
)

type Model struct {
	currentMode Status
	store       *store.Store
	cmds        []store.Command
	currentCmd  store.Command
	currentIdx  int
	textarea    textarea.Model
	cmdsHistory history.Model
	width       int
	height      int
}

func NewModel(db *store.Store) Model {
	cmds, err := db.GetCommands()
	if err != nil {
		log.Fatalf("Failed to get commands: %v", err)
	}

	currentIdx := len(cmds) - 1
	cmdsHistory := history.NewModel(cmds)
	cmdsHistory.Select(currentIdx)
	cmdsHistory.SetHeight(24, false)

	ta := textarea.New()
	ta.ShowLineNumbers = false
	ta.Prompt = ""
	ta.SetWidth(80 - 4 - 1 - 4 - 2)

	// Make textarea background transparent
	ta.FocusedStyle.Base = ta.FocusedStyle.Base.Background(lipgloss.NoColor{})
	ta.FocusedStyle.CursorLine = lipgloss.NewStyle()
	ta.FocusedStyle.Text = ta.FocusedStyle.Text.Background(lipgloss.NoColor{})
	ta.FocusedStyle.Placeholder = ta.FocusedStyle.Placeholder.Background(lipgloss.NoColor{})
	ta.FocusedStyle.EndOfBuffer = ta.FocusedStyle.EndOfBuffer.Background(lipgloss.NoColor{})

	ta.BlurredStyle.Base = ta.BlurredStyle.Base.Background(lipgloss.NoColor{})
	ta.BlurredStyle.CursorLine = lipgloss.NewStyle()
	ta.BlurredStyle.Text = ta.BlurredStyle.Text.Background(lipgloss.NoColor{})
	ta.BlurredStyle.Placeholder = ta.BlurredStyle.Placeholder.Background(lipgloss.NoColor{})
	ta.BlurredStyle.EndOfBuffer = ta.BlurredStyle.EndOfBuffer.Background(lipgloss.NoColor{})

	return Model{
		currentMode: ViewMode,
		store:       db,
		cmds:        cmds,
		currentCmd:  store.Command{},
		currentIdx:  currentIdx,
		textarea:    ta,
		cmdsHistory: cmdsHistory,
		width:       80, // Default width
		height:      24, // Default height
	}
}

func (m Model) Init() tea.Cmd {
	// Start ticker for rainbow animation
	return tickCmd()
}

func tickCmd() tea.Cmd {
	return tea.Tick(100*time.Millisecond, func(t time.Time) tea.Msg {
		return tickMsg(t)
	})
}

type tickMsg time.Time

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var (
		cmds []tea.Cmd
		cmd  tea.Cmd
	)

	switch msg := msg.(type) {
	case tickMsg:
		// Update rainbow animation
		m.cmdsHistory.UpdateAnimation()
		cmds = append(cmds, tickCmd())

	case tea.WindowSizeMsg:
		// Update terminal dimensions
		m.width = msg.Width
		m.height = msg.Height
		m.cmdsHistory.SetWidth(msg.Width)
		m.cmdsHistory.SetHeight(msg.Height, m.currentMode == NewCommandMode)
		m.textarea.SetWidth(msg.Width - 4 - 1 - 4 - 2)

	case tea.KeyMsg:
		key := msg.String()

		// Global keybindings
		switch key {
		case "ctrl+d":
			return m, tea.Quit
		}

		// Mode-dependant keybindings
		switch m.currentMode {
		case ViewMode:
			m.cmdsHistory, cmd = m.cmdsHistory.Update(msg)
			cmds = append(cmds, cmd)
			return HandleViewModeKey(m, key)
		case EditMode:
			switch key {
			case "ctrl+r", "esc":
				// Handle these without passing to textarea
				return HandleEditModeKey(m, key)
			default:
				// Pass other keys to the history's textarea
				m.cmdsHistory, cmd = m.cmdsHistory.Update(msg)
				cmds = append(cmds, cmd)
				return m, tea.Batch(cmds...)
			}
		case NewCommandMode:
			switch key {
			case "ctrl+r", "esc":
				// Handle these without passing to textarea
				return HandleNewCommandModeKey(m, key)
			default:
				// Pass other keys to textarea in new command mode
				m.textarea, cmd = m.textarea.Update(msg)
				cmds = append(cmds, cmd)
				return m, tea.Batch(cmds...)
			}
		}
	default:
		// Pass non-keyboard messages to both components
		m.textarea, cmd = m.textarea.Update(msg)
		cmds = append(cmds, cmd)

		m.cmdsHistory, cmd = m.cmdsHistory.Update(msg)
		cmds = append(cmds, cmd)
	}
	return m, tea.Batch(cmds...)
}

func HandleViewModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {

	// Create a new command
	case "n":
		m.currentMode = NewCommandMode
		m.currentIdx = -1
		m.currentCmd = store.Command{}
		m.textarea.SetValue("")
		m.textarea.Focus()
		m.cmdsHistory.ClearSelection()
		m.cmdsHistory.SetHeight(m.height, true)

	// Go one command up
	case "up", "k":
		if len(m.cmds) > 0 {
			if m.currentIdx == -1 {
				m.currentIdx = len(m.cmds) - 1
			} else if m.currentIdx > 0 {
				m.currentIdx -= 1
			}
			m.cmdsHistory.Select(m.currentIdx)
		}

	// Go one command down
	case "down", "j":
		if len(m.cmds) > 0 {
			if m.currentIdx == -1 {
				m.currentIdx = 0
			} else if m.currentIdx < len(m.cmds)-1 {
				m.currentIdx += 1
			}
			m.cmdsHistory.Select(m.currentIdx)
		}

	// Edit the current command inline
	case "enter":
		if m.currentIdx < 0 {
			return m, nil
		}
		m.currentMode = EditMode
		m.cmdsHistory.StartInlineEdit(m.currentIdx)
	}

	return m, nil
}

func HandleEditModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {
	// Save the inline edited command
	case "ctrl+r":
		command := m.cmdsHistory.GetEditedCommand()
		if command != "" && m.currentIdx >= 0 {
			m.currentCmd = m.cmds[m.currentIdx]
			m.currentCmd.Command = strings.TrimRight(command, "\r\n")

			var err error
			if err = m.store.SaveCommand(m.currentCmd); err != nil {
				log.Fatalf("Failed to save command to db: %v", err)
				return m, tea.Quit
			}

			m.cmds, err = m.store.GetCommands()
			if err != nil {
				log.Fatalf("Failed to get commands: %v", err)
				return m, tea.Quit
			}

			m.cmdsHistory.StopInlineEdit()
			m.cmdsHistory.SetCommands(m.cmds)
			m.cmdsHistory.Select(m.currentIdx)
			m.currentMode = ViewMode
		}

	// Cancel inline editing and return to view mode
	case "esc":
		m.cmdsHistory.StopInlineEdit()
		m.currentMode = ViewMode
	}

	return m, nil
}

func HandleNewCommandModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {
	// Register a new command
	case "ctrl+r":
		command := m.textarea.Value()
		if command != "" {
			m.currentCmd.Command = strings.TrimRight(command, "\r\n")

			// TODO: find a title for the command
			var err error
			if err = m.store.SaveCommand(m.currentCmd); err != nil {
				log.Fatalf("Failed to save command to db: %v", err)
				// TODO: maybe handle this more gracefully
				return m, tea.Quit
			}

			m.cmds, err = m.store.GetCommands()
			if err != nil {
				log.Fatalf("Failed to get commands: %v", err)
				// TODO: maybe handle this more gracefully
				return m, tea.Quit
			}

			m.currentCmd = store.Command{}
			m.currentIdx = -1
			m.currentMode = ViewMode
			m.cmdsHistory.SetCommands(m.cmds)
			m.cmdsHistory.Select(len(m.cmds) - 1)
			m.cmdsHistory.SetHeight(m.height, false)
		}

	// Cancel and return to view mode
	case "esc":
		m.currentMode = ViewMode
		m.cmdsHistory.SetHeight(m.height, false)
	}

	return m, nil
}
