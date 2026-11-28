---
id: ADR-2611261303
status: proposed
date: 2026-11-26
---
# ADR-2611261303: A Python port declares the API contract

**Status:** Proposed
**Date:** 2026-11-26
**Built:** all five decisions, 2026-11-26, 3eaa6ed (36 sealed scenarios)
**Drivers:** hexa reads Python (ADR-2611211324), and a Python project that tags its driving port with `@hexa:api` is told it has no tagged operations. The tags are ignored without a word about why: the parser returns nothing for Python.
**Supersedes:** none. Extends ADR-2610092245 to a fourth language.

## Context

ADR-2610092245 puts the API contract on a driving port, written once, read for Rust, Go and TypeScript. It builds one language-neutral model and renders OpenAPI 3.1 from it, so the languages can be held to one document. In Python a port is a class that declares methods (a `typing.Protocol` or an abstract base class), its types are written as annotations, and, as for `@hexa:public` (ADR-2611221644), a hexa tag is a `#` comment above what it marks.

## Decision

1. **The tag is a `#` comment above a class in the `ports` layer and above each exposed method.** The comment run directly above the class or method, or above its first decorator, holds the tag, with the grammar of ADR-2610092245 §1. The description is that run's prose with every `@hexa:` line removed; where the run has none, it is the method's docstring, cleaned as `inspect.cleandoc` cleans it. A method with no tag is not exposed. A tag inside a string or a docstring is not a tag. A tag on a class outside the `ports` layer is the rule error it is in the other languages (ADR-2610092245 §6). A tagged `@overload` or `@property` is an error.

2. **The signature is read from annotations.** The first parameter of a method that is not a `@staticmethod` is its receiver, whatever its name, and is not part of the API. A bare `*` or `/` is not a parameter. `def` and `async def` are read alike, and a return of `Awaitable[T]` or `Coroutine[_, _, T]` is `T`. A return of `None` is a `204` unless the tag gives another status. A parameter or a return with no annotation, and `*args` or `**kwargs`, is an error that names the file and line. A string annotation is read as the annotation it holds, and one that does not parse is an error; `from __future__ import annotations` changes nothing. Parameters are bound by the rules of ADR-2610092245 §2, and a default does not make one optional unless its type allows `None` and the default is `None`. A parameter whose type allows `None` without that default is an error, as such a field is.

3. **Errors are declared in comments.** Python has no closed error type, so, as for Go and TypeScript, a `@hexa:status <code> <text>` line in a method's comment run declares an error of that method, and one in the class's comment run applies to every operation of the port. Every Python method can raise, so its error channel is opaque and its operation declares `500`, as a TypeScript method's does.

4. **Schemas come from three kinds of class, and what cannot be read is an error.** A schema class is a `@dataclass`, a `TypedDict` or a Pydantic `BaseModel`, and each annotated field's name as written is its wire name. Names are resolved across the project as for the other languages: `from m import X as Y` makes `Y` another name for `X`, a dotted `m.X` names `X`, and `.pyi` files are not read for the API. `NewType` is the type it wraps. `str`, `int`, `float` and `bool` are string, integer, number and boolean. `list[T]`, `Sequence[T]` and `tuple[T, ...]`, and their `typing` spellings, are arrays; `dict[str, T]` and `Mapping[str, T]` are maps. `T | None`, `None | T`, `Optional[T]` and `Union[T, None]` are one thing. A field is not required when, in a dataclass or `BaseModel`, it is `T | None` with default `None`, or, in a `TypedDict`, it is `NotRequired[T]`, or the class is `total=False` and the field is not `Required[T]`; any other default leaves it required. The contract cannot say "present but may be null", so a field that allows `None` and is required is an error. `ClassVar`, `InitVar` and `KW_ONLY` annotations are not fields, whatever their name, and `model_config` is never a field.

   Each of these is an error naming the file and line of the use, or, for a refused schema class, of its declaration, never `{}` and never a guess: a `NamedTuple` (it is sent as an array); `Any`, a union of two types, a fixed-length tuple, a bare `list` or `dict`, `Enum`, `Literal`, `datetime`, `date`, `UUID`, `Decimal`, `set`, `bytes`, `Annotated` and any other class; a schema class with a base other than `BaseModel` or `TypedDict`, or a class decorator other than `@dataclass`, or a `@dataclass` given a `config` argument; a class keyword other than `total` on a `TypedDict` or `frozen` on a `BaseModel`; a field whose name begins with `_`; a field whose right-hand side is not a literal, `None`, or `field(...)` given only `default` or `default_factory`, or `Field(...)` given only those or `description`; a method decorated `computed_field`, `field_serializer` or `model_serializer`; and a `model_config` or `class Config` setting anything but `frozen`. These last rules are so a name or a field the server may not send is never published.

5. **Python joins the same contract.** Parsing produces the same `ApiContract`, so `hexa api spec`, `check` and `list`, the `hexa ci` gate and the grade's findings (ADR-2610092245 §6) treat a Python port as they treat the others. The four-operation bookmarks port written in Python, with fields spelled as the shared document spells them, renders that document. `hexa api` reads the files the grade reads (ADR-2611251938 §1); when it finds no operation and Python files were left ungraded, it says how many and how to have them graded.

## Consequences

- A Python project with a stray or unreadable `@hexa:api` tag loses grade for it, as the other languages do. Before this, the tag did nothing.
- A port that uses types outside §4 cannot be rendered until it is written in them. That is deliberate: a guessed schema is worse than none. Schema inheritance, `Enum`, `Literal` and dates are the ones a project is likeliest to meet; each can be added by its own decision.
- A Python port is satisfied by shape. A primary adapter that never names the port's class, in an annotation or an import, leaves it reported as unserved.
- TypeScript has only `number` (ADR-2610092245, Consequences), so a Python document equals a TypeScript one only where no integer appears.
- `hexa api adapter` still has no Python adapter and says so. Generating one means choosing a web framework, which ADR-2610092245 leaves to its own decision.

## Implementation

- `hexa-analysis/src/treesitter_api.rs`: tag reading, signature lowering and class reading for Python, into the `ApiFacts` of `hexa-analysis/src/domain.rs`, behind `AstPort::extract_api` in `hexa-analysis/src/ports.rs`.
- `hexa-analysis/src/api_contract.rs`: resolving Python names and aliases across the project's files.
- `hexa-cli/src/commands/api.rs`: the count of ungraded Python files when no operation is found.

**Gate**, written before the code: `hexa spec verify ADR-2611261303`, with a Python fixture held to the hand-written `expected.openapi.json` the other three languages already match.

## References

- ADR-2610092245 (the API contract is a tagged driving port), ADR-2611211324 (hexa reads Python), ADR-2611221644 (Python reads `@hexa:public`).
