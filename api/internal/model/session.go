package model

import (
	"time"

	"github.com/google/uuid"
)

type Session struct {
	ID            string     `json:"id"`
	UserID        uuid.UUID  `json:"user_id"`
	Tool          string     `json:"tool"`
	ProjectName   string     `json:"project_name,omitempty"`
	PromptCount   int        `json:"prompt_count"`
	DurationSecs  int        `json:"duration_seconds,omitempty"`
	StartedAt     time.Time  `json:"started_at"`
	EndedAt       *time.Time `json:"ended_at,omitempty"`
	Metadata      any        `json:"metadata,omitempty"`
	CreatedAt     time.Time  `json:"created_at"`
}

type SessionEvent struct {
	ID           int64     `json:"id"`
	SessionID    string    `json:"session_id"`
	EventType    string    `json:"event_type"`
	Content      string    `json:"content,omitempty"`
	ContextFiles any       `json:"context_files,omitempty"`
	Timestamp    time.Time `json:"timestamp"`
	Metadata     any       `json:"metadata,omitempty"`
}

type CommitLink struct {
	ID           int64     `json:"id"`
	SessionID    string    `json:"session_id"`
	CommitSHA    string    `json:"commit_sha"`
	RepoFullName string    `json:"repo_full_name"`
	CreatedAt    time.Time `json:"created_at"`
}
