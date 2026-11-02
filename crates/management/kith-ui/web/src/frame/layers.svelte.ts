// Frame layers that are not in the URL: the palette, the shortcut sheet, the header's
// Needs-you popover and the ExitChip's picker. The mobile Network sheet mirrors
// `?sheet=network` (App.svelte sets it), so Back closes it. Overview's `View all (5)` opens the
// same popover the header button does.

class Layers {
  palette = $state(false);
  shortcuts = $state(false);
  needs = $state(false);
  network = $state(false);
  /** The ExitChip's picker, also opened by ⌘K `Use exit…`. */
  exit = $state(false);

  /** True while any frame layer is open, so single-key shortcuts stay quiet. */
  get any(): boolean {
    return this.palette || this.shortcuts || this.needs || this.network || this.exit;
  }
}

export const layers = new Layers();
