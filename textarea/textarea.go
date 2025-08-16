package textarea

import (
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/lipgloss"
)

// New creates a new textarea with transparent background and standard configuration
func New() textarea.Model {
	ta := textarea.New()
	ta.ShowLineNumbers = false
	ta.Prompt = ""

	// Make textarea background transparent for focused state
	ta.FocusedStyle.Base = ta.FocusedStyle.Base.Background(lipgloss.NoColor{})
	ta.FocusedStyle.CursorLine = lipgloss.NewStyle()
	ta.FocusedStyle.Text = ta.FocusedStyle.Text.Background(lipgloss.NoColor{})
	ta.FocusedStyle.Placeholder = ta.FocusedStyle.Placeholder.Background(lipgloss.NoColor{})
	ta.FocusedStyle.EndOfBuffer = ta.FocusedStyle.EndOfBuffer.Background(lipgloss.NoColor{})

	// Make textarea background transparent for blurred state
	ta.BlurredStyle.Base = ta.BlurredStyle.Base.Background(lipgloss.NoColor{})
	ta.BlurredStyle.CursorLine = lipgloss.NewStyle()
	ta.BlurredStyle.Text = ta.BlurredStyle.Text.Background(lipgloss.NoColor{})
	ta.BlurredStyle.Placeholder = ta.BlurredStyle.Placeholder.Background(lipgloss.NoColor{})
	ta.BlurredStyle.EndOfBuffer = ta.BlurredStyle.EndOfBuffer.Background(lipgloss.NoColor{})

	return ta
}

// NewWithWidth creates a new textarea with the specified width
func NewWithWidth(width int) textarea.Model {
	ta := New()
	ta.SetWidth(width)
	return ta
}
