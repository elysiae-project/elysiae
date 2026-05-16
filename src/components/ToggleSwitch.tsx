import { useGame } from "../hooks/useGame";
import { ComponentSize, Variants } from "../types";
import { useState } from "preact/hooks";
import { cva } from "class-variance-authority";
import { Check, X } from "lucide-preact";

const toggleSwitchStyles = cva(
  "transition-colors duration-150 delay-0",
  {
    variants: {
      game: {
        [Variants.BH3]: "rounded-none",
        [Variants.HK4E]: "rounded-full border-2 border-white",
        [Variants.HKRPG]: "rounded-full",
        [Variants.NAP]: "rounded-full border-4 border-nap-dot-border",
      },
      size: {
        xs: "p-0.375",
        sm: "p-0.75",
        md: "p-1.5",
        lg: "p-3",
        xl: "p-6",
      },
      variant: {
        active: null,
        inactive: null,
      },
    },
    compoundVariants: [
      {
        game: Variants.BH3,
        variant: "inactive",
        class: "bg-bh3-toggle-inactive",
      },
      {
        game: Variants.BH3,
        variant: "active",
        class: "bg-bh3-toggle-active",
      },
      {
        game: Variants.HK4E,
        variant: "inactive",
        class: "bg-hk4e-toggle-inactive",
      },
      {
        game: Variants.HK4E,
        variant: "active",
        class: "bg-hk4e-toggle-active",
      },
      {
        game: Variants.HKRPG,
        variant: "inactive",
        class: "bg-hkrpg-toggle-inactive",
      },
      {
        game: Variants.HKRPG,
        variant: "active",
        class: "bg-hkrpg-toggle-active",
      },
      {
        game: Variants.NAP,
        variant: "inactive",
        class: "bg-nap-toggle-inactive",
      },
      {
        game: Variants.NAP,
        variant: "active",
        class: "bg-nap-toggle-active",
      },
    ],
  },
);

const toggleSwitchKnobStyles = cva(
  "duration-400 shrink-0 flex items-center justify-center",
  {
    variants: {
      game: {
        [Variants.BH3]: "bg-white border-2 border-bh3-knob-border",
        [Variants.HK4E]: "bg-hk4e-item-active-bg rounded-full",
        [Variants.HKRPG]: "bg-white rounded-full",
        [Variants.NAP]: "rounded-full bg-nap-knob-bg",
      },
    },
  },
);

const PADDING_REM: Record<ComponentSize, number> = {
  xs: 0.9375,
  sm: 0.1875,
  md: 0.375,
  lg: 0.75,
  xl: 1.5,
};

export default function ToggleSwitch({
  onClick,
  startActive = false,
  width = 7.5,
  height = 1.25,
  size = "md",
}: {
  onClick: (enabled: boolean) => void;
  startActive?: boolean;
  width?: number;
  height?: number;
  size?: ComponentSize;
}) {
  const { game } = useGame();
  const [enabled, setEnabled] = useState<boolean>(startActive);
  const padding = PADDING_REM[size];
  const knobSize = Math.max(height - padding * 2, 0.25);
  const travelDistance = Math.max(width - knobSize - padding * 2, 0);

  return (
    <div
      onClick={() => {
        setEnabled(!enabled);
        onClick(!enabled);
      }}
      style={{
        width: `${width}rem`,
        height: `${height}rem`,
      }}
      class={toggleSwitchStyles({
        game,
        size,
        variant: enabled ? "active" : "inactive",
      })}
    >
      <div
        style={{
          width: `${knobSize}rem`,
          height: `${knobSize}rem`,
          transform: `translateX(${enabled ? travelDistance : 0}rem) translateZ(1px)`,
          transition: "transform 0.175s ease",
        }}
        class={toggleSwitchKnobStyles({ game })}
      >
        {enabled ? (
          <Check size={knobSize * 0.6} />
        ) : (
          <X size={knobSize * 0.6} />
        )}
      </div>
    </div>
  );
}
