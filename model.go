package main

import (
	"log"

	"cahier/history"
	"cahier/store"
	"github.com/charmbracelet/bubbles/textarea"
	tea "github.com/charmbracelet/bubbletea"
)

type Status int

const (
	ViewMode Status = iota
	EditMode
)

type Model struct {
	currentMode Status
	store       *store.Store
	cmds        []store.Command
	currentCmd  store.Command
	currentIdx  int
	textarea    textarea.Model
	cmdsHistory history.Model
}

func NewModel(db *store.Store) Model {
	cmds, err := db.GetCommands()
	if err != nil {
		log.Fatalf("Failed to get commands: %v", err)
	}

	currentIdx := len(cmds) - 1
	cmdsHistory := history.NewModel(cmds)
	cmdsHistory.Select(currentIdx)

	return Model{
		currentMode: ViewMode,
		store:       db,
		cmds:        cmds,
		currentCmd:  store.Command{},
		currentIdx:  currentIdx,
		textarea:    textarea.New(),
		cmdsHistory: cmdsHistory,
	}
}

func (m Model) Init() tea.Cmd {
	return nil
}

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var (
		cmds []tea.Cmd
		cmd  tea.Cmd
	)

	m.textarea, cmd = m.textarea.Update(msg)
	cmds = append(cmds, cmd)

	switch msg := msg.(type) {
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
			return HandleViewModeKey(m, key)
		case EditMode:
			return HandleEditModeKey(m, key)
		}
	}
	return m, tea.Batch(cmds...)
}

func HandleViewModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {

	// Create a new command
	case "n":
		m.currentMode = EditMode
		m.currentIdx = -1
		m.currentCmd = store.Command{}
		m.textarea.SetValue("")
		m.textarea.Focus()

	// Go one command up
	case "up":
		if len(m.cmds) > 0 {
			if m.currentIdx == -1 {
				m.currentIdx = len(m.cmds) - 1
			} else if m.currentIdx > 0 {
				m.currentIdx -= 1
			}
			m.cmdsHistory.Select(m.currentIdx)
		}

	// Go one command down
	case "down":
		if len(m.cmds) > 0 {
			if m.currentIdx == -1 {
				m.currentIdx = 0
			} else if m.currentIdx < len(m.cmds)-1 {
				m.currentIdx += 1
			}
			m.cmdsHistory.Select(m.currentIdx)
		}

	// Edit the current command
	case "enter":
		if m.currentIdx < 0 {
			return m, nil
		}
		m.currentMode = EditMode
		m.currentCmd = m.cmds[m.currentIdx]
		m.textarea.SetValue(m.currentCmd.Command)
		m.textarea.Focus()
		m.textarea.CursorEnd()
	}

	return m, nil
}

func HandleEditModeKey(m Model, key string) (Model, tea.Cmd) {
	switch key {
	// Register a new command
	case "enter":
		command := m.textarea.Value()
		if command != "" {
			m.currentCmd.Command = command

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
		}

	// Cancel and return to view mode
	case "esc":
		m.currentMode = ViewMode
	}

	return m, nil
}
