// Dev-only entry for /__kit-overlays (`npm run dev`): every overlay component in each of its
// states, against the real tokens. Vite builds only index.html, so this never ships.

import '../../../../tailwind.css';
import { mount } from 'svelte';
import Overlays from './Overlays.svelte';

const target = document.getElementById('app');
if (target) mount(Overlays, { target });
