# game-ttt-go

Tic-tac-toe against a perfect opponent, in Go.

## Play it

```bash
./run.sh                    # you are X; enter a cell 0-8
./run.sh -demo              # perfect against perfect, then exit
```

The opponent uses minimax and cannot be beaten. The best you can get is a draw.
`-demo` plays both sides perfectly, so it always ends in a draw — that result
is an assertion about the search, not just proof the binary started.

## What is here

`board.go` holds the position and the rules. Its `cells` array is unexported on
purpose: no caller outside the package can build a board that no real game
could reach.

`minimax.go` is the search. `BestMove` returns the best cell for a player.

`cmd/ttt/main.go` is the only part that reads or prints. `Board.String` is a
wire format — exactly nine characters, round-tripping with `Parse` — so the
command draws the grid itself rather than making the library pretty.

## Test it

```bash
go test ./...
```
