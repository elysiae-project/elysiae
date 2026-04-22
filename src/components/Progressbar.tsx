import { cva } from "class-variance-authority";
import { Variants } from "../types";

const progressbarStyles = cva("h-5", {
  variants: {
    game: {
      [Variants.BH3]: "bg-bh3-progress-bg rounded-sm",
      [Variants.HK4E]: "rounded-full bg-hk4e-progress-bg",
      [Variants.HKRPG]: "bg-hkrpg-progress-bg",
      [Variants.NAP]:
        "border-2 border-nap-progress-border bg-nap-progress-bg rounded-full",
    },
  },
});

const progressbarContainerStyles = cva("h-full transition-all duration-300", {
  variants: {
    game: {
      [Variants.BH3]: "bg-bh3-pbar-fill rounded-sm",
      [Variants.HK4E]: "rounded-full bg-hk4e-pbar-fill",
      [Variants.HKRPG]: "bg-hkrpg-progress-bg",
      [Variants.NAP]:
        "bg-linear-to-r from-nap-pbar-from from-10% via-nap-pbar-via via-60% to-nap-pbar-to rounded-full",
    },
  },
});

export default function Progressbar({ progress, game }: { progress: number; game: Variants }) {
  return (
    <div class={progressbarStyles({ game })}>
      <div
        style={{ width: `${progress}%` }}
        class={progressbarContainerStyles({ game })}
      ></div>
    </div>
  );
}
