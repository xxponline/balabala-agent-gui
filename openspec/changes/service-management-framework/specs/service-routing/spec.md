# Spec Delta

## ADDED Requirements

### Requirement: Envelope and typed payload

All communication SHALL travel as an Envelope `{to, from, corr, kind,
payload}`. Payloads are typed by default (`Typed(Arc<dyn Any + Send + Sync>)`,
passed by pointer, no serialization); serialized forms SHALL be ordinary
message types, not Router branches. `from` SHALL carry only instance or actor
addresses; replies SHALL match `corr` in the pending table, never `from`.
`kind` SHALL NOT depend on `TypeId`. A failed downcast SHALL reject the
message into dead letters as `TypeMismatch`.

#### Scenario: Typed payload passes by pointer

- **WHEN** a participant sends a Typed payload through the bus
- **THEN** the receiving participant obtains the same `Arc` without any
  serialization step

#### Scenario: Downcast mismatch is rejected and observed

- **WHEN** a receiver's expected message type does not match the delivered
  payload's concrete type
- **THEN** no handler runs, the message lands in dead letters, and the
  failure is recorded as `TypeMismatch`

### Requirement: Two-level addressing

Messages SHALL be addressable by instance (`Route::Instance(id)`) or by
service name with a policy (`Route::Service(name, policy)`). `Direct(id)`
SHALL carry an instance id and assert that the instance belongs to the named
service, refusing delivery otherwise. `Any` SHALL select one `available`
instance of the named service.

#### Scenario: Instance direct delivery

- **WHEN** a participant sends to `Route::Instance(id)` of a loaded instance
- **THEN** the envelope is delivered to that instance's mailbox

#### Scenario: Direct with wrong service name is refused

- **WHEN** a participant sends to `Route::Service(name, Direct(id))` where
  instance `id` does not belong to service `name`
- **THEN** delivery is refused with a routing error instead of reaching the
  instance

#### Scenario: Any picks an available instance

- **WHEN** a service has multiple instances and a participant sends to
  `Route::Service(name, Any)`
- **THEN** the envelope is delivered to one `available` instance

### Requirement: Request/reply with router-held timeouts

An ask SHALL put an envelope with `corr` on the bus and SHALL complete via the
reply envelope, which the Router matches to its pending entry. The Router
SHALL own the deadline: on expiry it cancels the pending entry, records a dead
letter, and the caller receives `Err(Timeout)`. Runtime-driven participants
receive ask results through an executor-agnostic future; host-driven
participants submit with `corr` and collect the reply at their mailbox.

#### Scenario: Question and answer both leave a trace

- **WHEN** a participant asks a service and the service replies
- **THEN** the reply is matched by `corr`, both envelopes are observable in
  the message trace, and the caller receives the typed reply

#### Scenario: Timeout is an observable event

- **WHEN** no reply arrives before the ask deadline
- **THEN** the Router cancels the pending entry, records a dead letter, and
  the caller receives `Err(Timeout)`

#### Scenario: Host-driven participant collects replies by polling

- **WHEN** the GUI submits an ask with `corr` and the reply arrives
- **THEN** the reply lands in the GUI participant's mailbox and is observed
  on its next `poll()`

### Requirement: Fail-fast delivery with zero hidden buffering

When the target is not `available` (`loading`/`offline`/`error`), the Router
SHALL reject immediately: asks get `Err(TargetNotAvailable(state))`, tells
land in dead letters. A full per-outlet queue SHALL fail immediately
(`Err(DeadLetter(ChannelFull))` for asks, dead letter for tells), never
holding entries until timeouts. The Router SHALL NOT keep hidden queues for
unavailable or congested targets.

#### Scenario: Ask to a non-available target fails immediately

- **WHEN** a participant asks an instance in `error`
- **THEN** the ask returns `Err(TargetNotAvailable(error))` at once without
  waiting

#### Scenario: Tell to a non-available target dead-letters

- **WHEN** a participant tells an instance in `loading`
- **THEN** the envelope lands in dead letters immediately

#### Scenario: A full per-outlet queue fails immediately

- **WHEN** the target's per-outlet queue is full at delivery time
- **THEN** the delivery fails at once (asks get `Err(DeadLetter(ChannelFull))`)
  and the caller does not wait for the ask timeout

#### Scenario: GUI user operations report failures

- **WHEN** a user-triggered GUI operation is delivered to a service
- **THEN** it travels as an ask so any failure is returned to the user; tell
  is used only for fire-and-forget internal streams

### Requirement: Participant parity

All participants — host actors, the GUI, and plugin actors — SHALL
communicate through the same envelopes and the same Router with no privileged
participant.

#### Scenario: Plugin and host actors communicate identically

- **WHEN** a plugin actor and a host actor exchange messages
- **THEN** both use the same envelopes and Router paths with no special
  casing for either side

### Requirement: Mailbox polling for host-driven participants

A host-driven participant (GUI) SHALL participate as a pull-mode actor: it
owns a Route and a mailbox (the Router's per-outlet queue), exposed as a
`Listener` whose `poll()`/`drain()`/`snapshot()` run synchronously on the
caller's thread. Change notifications SHALL carry the snapshot `Arc`
in-band, so `snapshot()` needs no return trip; on-demand queries use corr
asks. The middleware SHALL NOT expose any callback-based API.

#### Scenario: GUI polls its mailbox on its own thread

- **WHEN** messages addressed to the GUI participant arrive
- **THEN** they land in its mailbox and are observed when the GUI next calls
  `poll()` on its own thread; no callback executes on runtime threads

#### Scenario: Snapshot needs no return trip

- **WHEN** a service publishes a data change to a GUI subscriber
- **THEN** the event carries the snapshot `Arc` and the subscriber's
  `snapshot()` returns that value without sending another message
