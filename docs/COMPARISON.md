# Comparison

Most tools for AI-assisted development sit at one of three points. hexa sits at a
fourth.

## Where the tools sit

| Approach | What it checks | What it cannot see |
|---|---|---|
| **Prompting** (an agent, a chat) | Nothing. The reviewer is you. | Everything |
| **Spec-driven** (write the spec, the agent implements) | Whether the code matches a document | Whether the document is still true |
| **Test-driven** (write the tests, the agent implements) | Whether the code runs | Whether the shape survived |
| **Gate-driven with an architecture grade** (hexa) | Whether it runs, and whether it is the shape you asked for | Whether the *intent* was right |

Each row down closes the gap the row above leaves open. The last gap stays
open. hexa does not decide what to build. It decides whether what was built
counts.

## The spec problem

A spec is prose. Prose cannot fail. Code drifts from a spec in silence because
nothing runs the spec. This repository's own decision records show the shape of
it: 140 ADR ids were cited across code and docs on 2026-09-15 and 113 of them
had no file, the most-cited one referenced 39 times. Not one raised an error,
because nothing read a citation against the ledger.

A gate is a command. It exits nonzero the moment it stops being true. That is
the whole difference between the second row and the fourth.

## The test problem

A test suite answers one question: does it run. An agent will produce a feature
whose tests pass and whose use case imports a database driver. Nothing fails.
The next change is harder, and the one after that is harder still.

An architecture grade answers the second question. It is a graph property, so a
number falls out of it, and a number can be a gate. `hexa scaffold --grade A`
fails the build below the floor.

## What hexa is not

It is not a framework, a runtime, or an orchestration layer. It is one binary
with no daemon and no database. The code generation is a frontier model. hexa
supplies the deterministic floor, the two gates, and the adversarial pass. It
turns a capable model into a disciplined one and does not replace it.
