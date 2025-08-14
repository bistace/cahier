package main

import (
	"log"

	"cahier/store"
	tea "github.com/charmbracelet/bubbletea"
)

func main() {
	store := &store.Store{}
	if err := store.Init("./cahier.db"); err != nil {
		log.Fatalf("Failed to initialize db: %v", err)
	}

	m := NewModel(store)
	p := tea.NewProgram(m)
	if _, err := p.Run(); err != nil {
		log.Fatalf("Failed to run the program: %v", err)
	}
}
