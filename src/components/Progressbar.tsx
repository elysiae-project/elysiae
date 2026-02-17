import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

const progressbarStyles = cva("h-5", {
	variants: {
		game: {
			[Variants.BH]: "bg-[#21364a] rounded-sm",
			[Variants.YS]: "rounded-full bg-[#242424]",
			[Variants.SR]: "bg-[#c0bebf]",
			[Variants.NAP]: "border-2 border-[#212222] bg-[#262626] rounded-full",
		},
	},
});

const progressbarContainerStyles = cva("h-full transition-all duration-300", {
	variants: {
		game: {
			[Variants.BH]: "bg-[#5fcaff] rounded-sm",
			[Variants.YS]: "rounded-full bg-[#f59f27]",
			[Variants.SR]: "bg-[#f3cb54]",
			[Variants.NAP]:
				"bg-linear-to-r from-[#4766fe] from-10% via-[#529aff] via-60% to-[#5ec6ff] rounded-full",
		},
	},
});

export default function Progressbar({ progress }: { progress: number }) {
	const activeGame = useGame();
	//if (activeGame === Variants.YS) {
	//	return <YsProgressbar progress={progress} />;
	//}
	return (
		<div class={progressbarStyles({ game: activeGame })}>
			<div
				style={{ width: `${progress}%` }}
				class={progressbarContainerStyles({ game: activeGame })}
			></div>
		</div>
	);
}
