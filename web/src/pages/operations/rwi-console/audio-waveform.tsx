// Audio Waveform / Spectrum Visualizer Component
// Uses HeroUI semantic colors (primary/success/warning/danger) only.

export type WaveformColor = 'primary' | 'success' | 'warning' | 'danger';

const COLOR_MAP: Record<WaveformColor, string> = {
  primary: 'bg-primary shadow-primary/50',
  success: 'bg-success shadow-success/50',
  warning: 'bg-warning shadow-warning/50',
  danger: 'bg-danger shadow-danger/50',
};

interface AudioWaveformProps {
  active: boolean;
  level?: number;
  color?: WaveformColor;
}

export function AudioWaveform({ active, level = 50, color = 'primary' }: AudioWaveformProps) {
  const barCount = 14;

  return (
    <div className="flex items-center justify-center gap-1 h-8 px-2 py-1 bg-content2 rounded-lg border border-default-100/20 backdrop-blur-sm">
      {Array.from({ length: barCount }).map((_, i) => {
        const factor = Math.sin((i + 1) * 0.7) * 0.5 + 0.5;
        const barHeightPercent = active
          ? Math.min(100, Math.max(15, level * factor + (i % 3) * 15))
          : 10;

        return (
          <div
            key={i}
            className={`w-1 rounded-full transition-all duration-150 ${active ? COLOR_MAP[color] : 'bg-default-600/40'}`}
            style={{
              height: `${barHeightPercent}%`,
              transitionDelay: `${i * 20}ms`,
            }}
          />
        );
      })}
    </div>
  );
}
