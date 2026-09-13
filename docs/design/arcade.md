# Context Ruins

Context Ruins is an original terminal platformer inside Flere. Four cosmetic
avatars explore three connected ruins: The Lost Prompt, The Context Vault and
The Merge Temple. Climb ladders, jump pits and patrolling sentinels, collect each
room's key and reach its door. Nine optional relics reward exploration. The game
uses original room layouts and procedural pixel art; it does not access a game
service, repository, shell, native chat or model.

Open **Arcade: Context Ruins** in Actions (NAV `&`), or **Play Context Ruins** in
the mascot's right-click menu. Choose Duck, Robot, Cat or Flere with 1–4 or arrows;
Enter starts. All four share the same collision body and abilities.

- Left/right or h/l: move. Up/down or k/j: climb a ladder. Down also drops through
  a ledge away from a ladder.
- Space: jump, with a short buffered landing window and forgiving edge jumps.
- P or Esc: pause. P or Enter resumes; Q exits from pause.
- R: return to the current checkpoint without losing a life or collected items.
- S: copy the visible game through Flere's existing screenshot flow.

There is no round timer. A fall or sentinel costs one of three lives; the player
returns to the room's checkpoint with brief protection. Keys and relics survive.
Checkpoint flags in later rooms shorten retries. Zero lives opens a retry screen;
completing all three rooms opens the finish screen. Enter returns to the avatar
picker. Backtracking through the left edge preserves each room's collected items.

The arena uses ordinary colored half-block cells and contains the moving avatar,
scenery, ladders, doors and hazards. Terminals as small as 40×20 are playable with
a following horizontal camera. Larger terminals show more of the room. Resizing
pauses; enlarging does not resume automatically. Reduced motion freezes optional
scenery and picker animation while preserving the player's movement and hazards.
Focus loss pauses without an automatic focus-gain resume.

Arcade negotiates the Kitty keyboard protocol while open. Terminals that support
its event reports track movement and jump separately: holding left/right keeps
moving through a jump, and releasing the direction stops it. Pause, focus loss
and room transitions clear held controls; a fresh press starts them again.
The previous keyboard mode is restored when Arcade closes, the UI refreshes, or
the SSH companion disconnects or opens a local prompt.

Terminals without event reports use 180 ms key-repeat intent on the ground and
retain the launch direction in the air, so pressing Space does not interrupt a
jump. This fallback cannot detect a physical release mid-jump. Physics uses fixed
120 Hz steps with at most 100 ms of catch-up per UI tick, so an unresponsive UI
cannot silently simulate seconds of danger.

The attachment-local overlay preserves the existing native draft and selected
session. Native output continues to drain behind it. Opening and closing consume
the entire input burst, including fragmented paste/escape tails, UTF-8 tails and
captured pointer releases. Pasted text never becomes game commands. Screenshot
and native attachment guards treat the overlay as modal. Higher-priority local
forms pause the game. Exit restores the previous navigation mode and clears
owned graphics through the existing renderer. No game state is persisted.

Pure tests cover a complete three-room movement witness, jump/landing and key
expiry, ladders and gates, hazard/checkpoint lives, collected progress, pause and
cosmetic equivalence. Renderer tests cover the minimum camera, wide rooms,
selection and pause/results. Real disposable PTY tests exercise movement, jump,
relic collection, held movement and release, resize/focus pause, keyboard-mode
restoration and exact draft preservation. A real remote
bridge test verifies screenshots against the game cell canvas and rejects native
attachment offers without touching a real clipboard.
