import { useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

import { cva } from "class-variance-authority";
import { Check, X } from "lucide-preact";

const toggleSwitchStyles = cva(
	"w-25 p-1.5 transition-colors duration-150 delay-0",
	{
		variants: {
			game: {
				[Variants.BH]: "rounded-none",
				[Variants.YS]: "rounded-full border-2 border-white",
				[Variants.SR]: "rounded-full",
				[Variants.NAP]: "rounded-full border-4 border-[#353535]",
			},
			variant: {
				active: "",
				inactive: "",
			},
		},
		compoundVariants: [
			{
				game: Variants.BH,
				variant: "inactive",
				class: "bg-[#1e2035]",
			},
			{
				game: Variants.BH,
				variant: "active",
				class: "bg-[#313458]",
			},
			{
				game: Variants.YS,
				variant: "inactive",
				class: "bg-[#353d4f]",
			},
			{
				game: Variants.YS,
				variant: "active",
				class: "bg-[#dccba9]",
			},
			{
				game: Variants.SR,
				variant: "inactive",
				class: "bg-[rgba(49,49,49,0.40)]",
			},
			{
				game: Variants.SR,
				variant: "active",
				class: "bg-[#f5c76f]",
			},
			{
				game: Variants.NAP,
				variant: "inactive",
				class: "bg-[#242424]",
			},
			{
				game: Variants.NAP,
				variant: "active",
				class: "bg-[#a2a2a2]",
			},
		],
	},
);

const toggleSwitchKnobStyles = cva(
	"w-8 min-h-8 duration-400 flex items-center justify-center",
	{
		variants: {
			game: {
				[Variants.BH]: "bg-white border-2 border-[#c0bfc2]",
				[Variants.YS]: "bg-[#ece5d8] rounded-full",
				[Variants.SR]: "bg-white rounded-full",
				[Variants.NAP]: "rounded-full bg-[#575556]",
			},
		},
	},
);

export default function ToggleSwitch({
	onClick,
	startActive = false,
}: {
	onClick: (enabled: boolean) => void;
	startActive: boolean;
}) {
	const activeGame = useGame();
	let [enabled, setEanbled] = useState<boolean>(startActive);

	return (
		<div
			onClick={() => {
				setEanbled(!enabled);
				onClick(enabled);
			}}
			class={toggleSwitchStyles({
				game: activeGame,
				variant: `${enabled ? "active" : "inactive"}`,
			})}
		>
			<div
				style={{
					transform: `${enabled ? "translateX(160%)" : ""} translateZ(1px)`,
				}}
				class={toggleSwitchKnobStyles({ game: activeGame })}
			>
				{enabled ? <Check size={18} /> : <X size={18} />}
			</div>
		</div>
	);
}
