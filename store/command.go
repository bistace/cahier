package store

const (
	StatusRunning = "running"
	StatusSuccess = "success"
	StatusFailed  = "failed"
)

type Command struct {
	ID         int64
	Command    string
	Status     string
	ReturnCode int
}
