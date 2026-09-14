// Command ttt plays tic-tac-toe against the perfect minimax opponent.
//
// The package had the board, the rules and an unbeatable player, and no way to
// play against it. `go test ./...` was green and there was nothing to start.
//
// With -demo the computer plays both sides from a seed-free perfect strategy,
// which always draws, and the last line says so. That is what a gate reads.
package main

import (
	"bufio"
	"flag"
	"fmt"
	"os"
	"strconv"
	"strings"

	gamettt "gamettt"
)

// grid draws the position for a person.
//
// Board.String is a wire format — exactly nine characters, round-tripping with
// Parse — so it is the library's business and stays as it is. Making it pretty
// would break the format its own tests depend on.
func grid(b gamettt.Board) string {
	s := b.String()
	var out strings.Builder
	for row := 0; row < 3; row++ {
		out.WriteString("  ")
		for col := 0; col < 3; col++ {
			i := row*3 + col
			c := s[i]
			if c == '.' {
				out.WriteByte(byte('0' + i))
			} else {
				out.WriteByte(c)
			}
			if col < 2 {
				out.WriteString(" | ")
			}
		}
		out.WriteByte('\n')
		if row < 2 {
			out.WriteString("  ---------\n")
		}
	}
	return out.String()
}

func outcome(b gamettt.Board) string {
	switch w := b.Winner(); {
	case w == gamettt.X:
		return "GAME OVER — X wins"
	case w == gamettt.O:
		return "GAME OVER — O wins"
	case b.Full():
		return "GAME OVER — a draw"
	}
	return ""
}

// demo plays perfect against perfect. Two perfect players always draw, so this
// is a real assertion about the minimax, not just proof the binary started.
func demo() int {
	var b gamettt.Board
	fmt.Println("Perfect play, both sides.")
	for outcome(b) == "" {
		p := b.Turn()
		move, err := b.BestMove(p)
		if err != nil {
			fmt.Fprintln(os.Stderr, "no move:", err)
			return 1
		}
		if err := b.Move(move, p); err != nil {
			fmt.Fprintln(os.Stderr, "illegal move:", err)
			return 1
		}
	}
	fmt.Print(grid(b))
	fmt.Println(outcome(b))
	return 0
}

func main() {
	d := flag.Bool("demo", false, "play perfect against perfect and exit")
	flag.Parse()
	if *d {
		os.Exit(demo())
	}

	var b gamettt.Board
	in := bufio.NewScanner(os.Stdin)
	fmt.Println("Tic-tac-toe. You are X. Enter a cell 0-8, or Ctrl-D to quit.")
	for outcome(b) == "" {
		fmt.Print(grid(b))
		if b.Turn() == gamettt.X {
			fmt.Print("your move [0-8]: ")
			if !in.Scan() {
				fmt.Println("\nGAME OVER — you quit")
				return
			}
			pos, err := strconv.Atoi(strings.TrimSpace(in.Text()))
			if err != nil {
				fmt.Println("that is not a number 0-8")
				continue
			}
			if err := b.Move(pos, gamettt.X); err != nil {
				fmt.Println(err)
				continue
			}
			continue
		}
		move, err := b.BestMove(gamettt.O)
		if err != nil {
			fmt.Fprintln(os.Stderr, "no move:", err)
			return
		}
		if err := b.Move(move, gamettt.O); err != nil {
			fmt.Fprintln(os.Stderr, "illegal move:", err)
			return
		}
		fmt.Printf("O plays %d\n", move)
	}
	fmt.Print(grid(b))
	fmt.Println(outcome(b))
}
