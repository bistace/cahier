package main

import (
	"log"
	"strings"
	"time"

	"cahier/executor"
	"cahier/history"
	"cahier/store"

	ta "cahier/textarea"
	"github.com/charmbracelet/bubbles/textarea"
	tea "github.com/charmbracelet/bubbletea"
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

	textarea := ta.NewWithWidth(80 - 4 - 1 - 4 - 2)

	return Model{
		currentMode: ViewMode,
		store:       db,
		cmds:        cmds,
		currentCmd:  store.Command{},
		currentIdx:  currentIdx,
		textarea:    textarea,
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

type execStartMsg struct {
	cmdID int64
}

type execCompleteMsg struct {
	cmdID    int64
	exitCode int
}

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
			case "ctrl+r", "ctrl+s", "esc":
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

	case execStartMsg:
		// Update command status to running
		for i, cmd := range m.cmds {
			if cmd.ID == msg.cmdID {
				m.cmds[i].Status = store.StatusRunning
				m.cmds[i].ReturnCode = 0
				m.store.UpdateCommandStatus(msg.cmdID, store.StatusRunning, 0)
				m.cmdsHistory.SetCommands(m.cmds)
				break
			}
		}
	
	case execCompleteMsg:
		// Update command status based on exit code
		status := store.StatusSuccess
		if msg.exitCode != 0 {
			status = store.StatusFailed
		}
		
		for i, cmd := range m.cmds {
			if cmd.ID == msg.cmdID {
				m.cmds[i].Status = status
				m.cmds[i].ReturnCode = msg.exitCode
				m.store.UpdateCommandStatus(msg.cmdID, status, msg.exitCode)
				m.cmdsHistory.SetCommands(m.cmds)
				break
			}
		}
	
	default:
		// Pass non-keyboard messages to components
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
	// Save the inline edited command without running it
	case "ctrl+s":
		return saveCommand(m), nil

	// Save the inline edited command and run it
	case "ctrl+r":
		return saveAndRunCommand(m)

	// Cancel inline editing and return to view mode
	case "esc":
		m.cmdsHistory.StopInlineEdit()
		m.currentMode = ViewMode
	}

	return m, nil
}

func HandleNewCommandModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {
	// Register a new command and run it
	case "ctrl+r":
		return saveAndRunCommand(m)

	// Cancel and return to view mode
	case "esc":
		m.currentMode = ViewMode
		m.cmdsHistory.SetHeight(m.height, false)
	}

	return m, nil
}

// Save the command to the database and switch to viewMode
func saveCommand(m Model) Model {
	var command string
	var err error

	switch m.currentMode {
	case EditMode:
		// Get command from inline edit
		command = m.cmdsHistory.GetEditedCommand()
		if command != "" && m.currentIdx >= 0 {
			m.currentCmd = m.cmds[m.currentIdx]
			m.currentCmd.Command = strings.TrimRight(command, "\r\n")

			if err = m.store.SaveCommand(m.currentCmd); err != nil {
				log.Fatalf("Failed to save command to db: %v", err)
				return m
			}

			m.cmds, err = m.store.GetCommands()
			if err != nil {
				log.Fatalf("Failed to get commands: %v", err)
				return m
			}

			m.cmdsHistory.StopInlineEdit()
			m.cmdsHistory.SetCommands(m.cmds)
			m.cmdsHistory.Select(m.currentIdx)
			m.currentMode = ViewMode
		}

	case NewCommandMode:
		// Get command from textarea
		command = m.textarea.Value()
		if command != "" {
			m.currentCmd = store.Command{
				ID:      0, // Will be set by SaveCommand
				Command: strings.TrimRight(command, "\r\n"),
			}

			if err = m.store.SaveCommand(m.currentCmd); err != nil {
				log.Fatalf("Failed to save command to db: %v", err)
				return m
			}

			m.cmds, err = m.store.GetCommands()
			if err != nil {
				log.Fatalf("Failed to get commands: %v", err)
				return m
			}

			// Select the newly added command (last one)
			m.currentIdx = len(m.cmds) - 1
			m.cmdsHistory.SetCommands(m.cmds)
			m.cmdsHistory.Select(m.currentIdx)
			m.cmdsHistory.SetHeight(m.height, false)
			m.currentMode = ViewMode
			m.textarea.SetValue("")
			m.textarea.Blur()
		}
	}

	return m
}

// Execute command and return a tea.Cmd that will run it asynchronously
func executeCommand(cmdID int64, command string) tea.Cmd {
	return func() tea.Msg {
		result := executor.ExecuteCommand(command)
		return execCompleteMsg{
			cmdID:    cmdID,
			exitCode: result.ExitCode,
		}
	}
}

// Save and run the command
func saveAndRunCommand(m Model) (Model, tea.Cmd) {
	m = saveCommand(m)
	
	// Find the command that was just saved/updated and execute it
	if m.currentMode == ViewMode && m.currentIdx >= 0 && m.currentIdx < len(m.cmds) {
		cmd := m.cmds[m.currentIdx]
		// Update command status to running
		for i, c := range m.cmds {
			if c.ID == cmd.ID {
				m.cmds[i].Status = store.StatusRunning
				m.cmds[i].ReturnCode = 0
				m.store.UpdateCommandStatus(cmd.ID, store.StatusRunning, 0)
				m.cmdsHistory.SetCommands(m.cmds)
				break
			}
		}
		// Return the async command execution
		return m, executeCommand(cmd.ID, cmd.Command)
	}
	
	return m, nil
}
