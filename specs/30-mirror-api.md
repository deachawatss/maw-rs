# SPEC-30: Native mirror API route

Issue: #30
Date: 2026-07-11
Mode: standard

## Objective

Let `maw overview` read live content from a local tmux pane through the native
`maw-rs serve` HTTP backend, without relying on maw-js.

## Acceptance Criteria

- [ ] `GET /api/mirror?target=<session:pane>&lines=<N>` returns the captured
  pane text as a successful plain-text response.
- [ ] Omitted `lines` passes the known default of 50 to the tmux capture seam.
- [ ] A capture failure, including a missing pane, returns HTTP 404 and a clear
  JSON error response.
- [ ] The endpoint is mounted on the native `serve_router` used by `maw serve`.

## Seams and Testing

- HTTP boundary: `GET /api/mirror`, exercised through `serve_router` with the
  injected `ServeDelivery` tmux adapter.
- Prior art: existing `serve_tests` router tests using `tower::ServiceExt`.
- Expected values: issue contract literals `50`, `200`, and `404`; pane output
  is an independent known string from the test adapter.

## Decisions

### Response format and errors

- Chose: return raw `text/plain` on success so the existing `maw overview`
  `curl` command renders pane text directly; return structured JSON on failure.
- Why: overview embeds the response directly in `watch`, while error JSON gives
  callers a machine-readable, clear failure reason.
- Rejected: JSON success payload, because it would render JSON rather than the
  terminal content in overview panes.

## API Contract

- Request: `GET /api/mirror?target=<session:pane>&lines=<u32>`.
- `target` is forwarded unchanged to `ServeDelivery::capture_tail`.
- `lines` defaults to `50` when absent.
- Success: `200 OK`, `Content-Type: text/plain; charset=utf-8`, response body
  equal to the capture result.
- Capture failure: `404 Not Found`, JSON `{ "error": "pane_not_found",
  "target": "<target>", "message": "<capture error>" }`.

## Boundaries

- Always: use the existing `ServeDelivery::capture_tail` adapter; no direct
  tmux command in the route.
- Never: add a maw-js backend or change the existing `/api/capture` contract.
