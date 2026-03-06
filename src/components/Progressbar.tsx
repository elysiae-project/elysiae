import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

const progressbarStyles = cva("h-5", {
	variants: {
		game: {
			[Variants.BH]: "bg-bh-progress-bg rounded-sm",
			[Variants.YS]: "rounded-full bg-[#242424]",
			[Variants.SR]: "bg-sr-progress-bg",
			[Variants.NAP]:
				"border-2 border-nap-progress-border bg-nap-progress-bg rounded-full",
		},
	},
});

const progressbarContainerStyles = cva("h-full transition-all duration-300", {
	variants: {
		game: {
			[Variants.BH]: "bg-bh-pbar-fill rounded-sm",
			[Variants.YS]: "rounded-full bg-ys-pbar-fill",
			[Variants.SR]: "bg-sr-pbar-fill",
			[Variants.NAP]:
				"bg-linear-to-r from-nap-pbar-from from-10% via-nap-pbar-via via-60% to-nap-pbar-to rounded-full",
		},
	},
});

export default function Progressbar({ progress }: { progress: number }) {
	const { game } = useGame();

	return (
		<div class={progressbarStyles({ game: game })}>
			<div
				style={{ width: `${progress}%` }}
				class={progressbarContainerStyles({ game: game })}
			>
			</div>
		</div>
	);
}
