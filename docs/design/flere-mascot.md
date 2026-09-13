# Flere screensaver mascot

The default mascot is inspired by Flere-Imsaho in Iain M. Banks's
*The Player of Games*. The workbench now shares its short name, Flere. The screensaver keeps
the original animated Flere intro, with a small Flere drifting around beneath
the wordmark. The sidebar pet uses the same character and emotional fields.

New or missing mascot preferences select Flere; existing explicit choices are
retained. The Actions menu's **Mascot: Flere (screensaver + pet)** choice sets both
characters and immediately previews the intro. It preserves whether the sidebar
pet is shown. Duck remains available, and the pet controls retain Robot and Cat.

Flere does not bundle the novel, an EPUB, source-text extracts or reference
boards. The appearance below is a visual interpretation, with attribution to
the fictional inspiration.

## Appearance and emotional fields

The book describes a small white circular drone, separate revolving outer
sections around a stationary core, and fields resembling an exotic insect's
wings. The terminal version emphasizes the white disc and revolving rings,
without inventing eyes, a mouth or limbs. It renders those fields as a close halo rather than separate wings. Ring count, viewing
angle, shading, field geometry and animation timing are visual interpretations
rather than measurements from the text.

| State | Field treatment | Basis |
| --- | --- | --- |
| Resting | Yellow-green, gently moving halo | The book associates this colour with mellow approachability; the compact halo is a visual adaptation. |
| Enjoying a pat | Warm orange-red glow and a small wobble | Orange wellbeing and red pleasure/amusement inform this adaptation; petting itself is invented interaction. |
| Annoyed by a pat | Brief white flecks and an indignant tilt | White irritation flecks are described in the book; the gesture and duration are interpretation. |

The white body remains distinct from the changing fields. Red is not used as
an alarm indicator for this character. Other story colours may inform future
states, but are not automatically treated as universal UI status colours.

Both the sidebar pet and the wandering intro mascot emphasize the solid disc
and rings, with a close emotion halo. Compressing the fields avoids an insect
silhouette in either view while retaining the original intro's wordmark and rails.

## Interaction

A press and release on the mascot without a meaningful drag counts as a pat.
Either satisfaction or annoyance may follow; the reaction is random, then
settles back to the resting field. Hovering alone does not trigger a reaction.
Dragging grabs the mascot, and releasing after a drag throws it within the
terminal's bounds.

Mouse movement, clicking and playing keep the screensaver open. Keyboard input
or a paste wakes Flere, and the waking input is consumed rather than entering
the active shell or chat. Sessions continue receiving output while the overlay
is visible. Reduced motion keeps feedback visible without continuous animation.

The renderer uses Flere's own terminal cells and half-block raster drawing.
It requires neither a desktop overlay nor a terminal image protocol. Physical
terminal and platform acceptance remain separate from unit tests and compile
checks.
