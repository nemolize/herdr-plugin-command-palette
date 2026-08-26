// Prototype palette: input line, filtered list, keyboard nav, herdr JSON.
// Scope-matched to prototypes/rs so the two can be compared.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strings"

	tea "charm.land/bubbletea/v2"
)

type candidate struct {
	title string
	id    string
	score int
}

type paneList struct {
	Result struct {
		Panes []struct {
			PaneID         string `json:"pane_id"`
			TerminalTitle  string `json:"terminal_title_stripped"`
			WorkspaceID    string `json:"workspace_id"`
		} `json:"panes"`
	} `json:"result"`
}

func loadCandidates() ([]candidate, error) {
	out, err := exec.Command("herdr", "pane", "list").Output()
	if err != nil {
		return nil, err
	}
	var pl paneList
	if err := json.Unmarshal(out, &pl); err != nil {
		return nil, err
	}
	cands := make([]candidate, 0, len(pl.Result.Panes))
	for _, p := range pl.Result.Panes {
		t := p.TerminalTitle
		if t == "" {
			t = p.PaneID
		}
		cands = append(cands, candidate{title: t, id: p.PaneID})
	}
	return cands, nil
}

// Subsequence match; score rewards earlier and tighter matches.
func fuzzy(query, target string) (int, bool) {
	if query == "" {
		return 0, true
	}
	q, t := strings.ToLower(query), strings.ToLower(target)
	qi, score, prev := 0, 0, -1
	for ti := 0; ti < len(t) && qi < len(q); ti++ {
		if t[ti] == q[qi] {
			if prev >= 0 && ti == prev+1 {
				score += 5
			}
			score -= ti / 10
			prev = ti
			qi++
		}
	}
	if qi < len(q) {
		return 0, false
	}
	return score, true
}

type model struct {
	all      []candidate
	filtered []int
	query    string
	sel      int
	chosen   string
	err      error
}

func (m *model) refilter() {
	m.filtered = m.filtered[:0]
	scored := make([]candidate, 0, len(m.all))
	for i, c := range m.all {
		if s, ok := fuzzy(m.query, c.title); ok {
			scored = append(scored, candidate{title: c.title, id: c.id, score: s*1000 - i})
		}
	}
	sort.SliceStable(scored, func(a, b int) bool { return scored[a].score > scored[b].score })
	for _, s := range scored {
		for i, c := range m.all {
			if c.id == s.id {
				m.filtered = append(m.filtered, i)
				break
			}
		}
	}
	if m.sel >= len(m.filtered) {
		m.sel = max(0, len(m.filtered)-1)
	}
}

func (m model) Init() tea.Cmd { return nil }

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyPressMsg:
		switch msg.String() {
		case "esc", "ctrl+c":
			return m, tea.Quit
		case "up", "ctrl+p":
			if m.sel > 0 {
				m.sel--
			}
		case "down", "ctrl+n":
			if m.sel < len(m.filtered)-1 {
				m.sel++
			}
		case "enter":
			if len(m.filtered) > 0 {
				m.chosen = m.all[m.filtered[m.sel]].id
			}
			return m, tea.Quit
		case "backspace":
			if len(m.query) > 0 {
				m.query = m.query[:len(m.query)-1]
				m.refilter()
			}
		default:
			if msg.Text != "" {
				m.query += msg.Text
				m.refilter()
			}
		}
	}
	return m, nil
}

func (m model) View() tea.View {
	var b strings.Builder
	fmt.Fprintf(&b, "> %s\n\n", m.query)
	for i, idx := range m.filtered {
		if i >= 10 {
			break
		}
		prefix := "  "
		if i == m.sel {
			prefix = "▶ "
		}
		fmt.Fprintf(&b, "%s%s\n", prefix, m.all[idx].title)
	}
	if len(m.filtered) == 0 {
		b.WriteString("  (no match)\n")
	}
	fmt.Fprintf(&b, "\n%d/%d · esc to close", len(m.filtered), len(m.all))
	return tea.NewView(b.String())
}

func main() {
	cands, err := loadCandidates()
	if err != nil {
		fmt.Fprintln(os.Stderr, "failed to load candidates:", err)
		os.Exit(1)
	}
	m := model{all: cands}
	m.refilter()
	final, err := tea.NewProgram(m).Run()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if fm, ok := final.(model); ok && fm.chosen != "" {
		fmt.Println(fm.chosen)
	}
}
