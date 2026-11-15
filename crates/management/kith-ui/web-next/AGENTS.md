# Notes for agents

Before using ui-sdk, follow the ui-sdk skill (node_modules/@ui-sdk/core/skills/ui-sdk) and read
node_modules/@ui-sdk/react/docs/llms.txt. To build a new component, also search
node_modules/@ui-sdk/react/docs/core.md by heading, and read rendering.md next to it.

Look: ui-sdk's default styles (`@ui-sdk/core/web.css`, imported in src/main.tsx before
src/styles.css) themed pure black and white, square corners, Geist. Use the components'
built-in styles rather than restyling them by hand.
