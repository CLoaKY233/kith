// One shared Canvas 2D painter per list (spec §5 Spark). Workstream F owns it.

import type { Palette } from '../tokens';

export interface SparkPainter {
  /** Redraws every registered spark, in the tick's rAF. */
  paint(palette: Palette): void;
  register(key: string, canvas: HTMLCanvasElement): () => void;
}

export function createSparkPainter(): SparkPainter {
  const canvases = new Map<string, HTMLCanvasElement>();
  return {
    paint(palette) {
      for (const canvas of canvases.values()) {
        const ctx = canvas.getContext('2d');
        if (!ctx) continue;
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.strokeStyle = palette['ink-4'];
        ctx.setLineDash([2, 2]);
        ctx.beginPath();
        ctx.moveTo(0, canvas.height - 0.5);
        ctx.lineTo(canvas.width, canvas.height - 0.5);
        ctx.stroke();
      }
    },
    register(key, canvas) {
      canvases.set(key, canvas);
      return () => canvases.delete(key);
    },
  };
}
