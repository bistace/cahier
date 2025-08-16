package store

import (
	"database/sql"
	"time"

	_ "github.com/mattn/go-sqlite3"
)

type Store struct {
	conn *sql.DB
}

func (s *Store) Init(dbPath string) error {
	var err error
	s.conn, err = sql.Open("sqlite3", dbPath)
	if err != nil {
		return err
	}

	query := `CREATE TABLE IF NOT EXISTS commands (
		id integer not null primary key,
		command text not null,
		status text default '',
		return_code integer default 0
	);`

	if _, err = s.conn.Exec(query); err != nil {
		return err
	}

	return nil
}

func (s *Store) GetCommands() ([]Command, error) {
	rows, err := s.conn.Query("SELECT id, command, status, return_code FROM commands")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	cmds := []Command{}
	for rows.Next() {
		var cmd Command
		rows.Scan(&cmd.ID, &cmd.Command, &cmd.Status, &cmd.ReturnCode)
		cmds = append(cmds, cmd)
	}

	return cmds, nil
}

func (s *Store) SaveCommand(cmd Command) error {
	if cmd.ID == 0 {
		cmd.ID = time.Now().UTC().UnixNano()
	}

	query := `INSERT INTO commands (id, command, status, return_code)
		VALUES (?, ?, ?, ?)
		ON CONFLICT(id) DO UPDATE
		SET command=excluded.command,
		    status=excluded.status,
		    return_code=excluded.return_code;`

	if _, err := s.conn.Exec(query, cmd.ID, cmd.Command, cmd.Status, cmd.ReturnCode); err != nil {
		return err
	}

	return nil
}
