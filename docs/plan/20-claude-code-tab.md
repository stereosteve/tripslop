# 20. Claude in the Code tab

Arkestra Studio's chat writes and edits ISF shaders through a tool call, keeps an undo for each
change, and swaps the result into the running chain. It runs on their servers and is metered
per month. tripslop already has the hard part: a live compiler that checks code through naga
before it reaches the GPU and reports errors with line numbers. That makes a natural tool loop
for Claude.

## Plan

* A **Chat** pane beside the editor in the device panel's Code tab. It's enabled when
  `ANTHROPIC_API_KEY` is set (or a key is pasted into the pane and kept in the config dir).
  Without a key it shows how to get one. Nothing is sent anywhere unless a key is set and you
  press send.
* Uses the Messages API with the latest Claude model and one tool, `set_shader(code)`. tripslop
  compiles the code and returns either *ok* plus the list of controls it found, or the compile
  errors with line numbers. Claude keeps going until it compiles or gives up after a few tries.
  The system prompt describes tripslop's ISF dialect from the README (built-ins, audio
  uniforms, `iChannel` meanings, naga's strictness).
* Each successful change is pushed onto an undo stack for that shader (*Undo last change*).
  Parameter values the user set are kept across changes when the control names match.
* An optional **screenshot** of the shader's output after each change is sent back with the
  tool result, so Claude can see what it made ("make the rings thinner").
* The conversation is saved next to the shader (or in the set, after 19).
* HTTP goes through a blocking client on a worker thread (`ureq`), with responses streamed into
  the pane. The UI thread never waits.

## Scripts and tests

The tool loop is tested against a fake client that returns canned tool calls: a broken shader,
then a fixed one. The test asserts that the error was fed back and the second version is the
one running. No network in tests.

## Done when

* "Make a tunnel of neon hexagons that pulses on the kick" gives a working generator in the cell
  you asked from, and "slower" makes it slower.
