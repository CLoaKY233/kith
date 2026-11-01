// Frame layers that are not in the URL: the palette, the shortcut sheet, the header's
// Needs-you popover and the mobile Network sheet. Overview's `View all (5)` opens the
// same popover the header button does.

class Layers {
  palette = $state(false);
  shortcuts = $state(false);
  needs = $state(false);
  network = $state(false);

  /** True while any frame layer is open, so single-key shortcuts stay quiet. */
  get any(): boolean {
    return this.palette || this.shortcuts || this.needs || this.network;
  }
}

export const layers = new Layers();
