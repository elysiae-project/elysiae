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
				class: "bg-bh-toggle-inactive",
			},
			{
				game: Variants.BH,
				variant: "active",
				class: "bg-bh-toggle-active",
			},
			{
				game: Variants.YS,
				variant: "inactive",
				class: "bg-ys-toggle-inactive",
			},
			{
				game: Variants.YS,
				variant: "active",
				class: "bg-ys-toggle-active",
			},
			{
				game: Variants.SR,
				variant: "inactive",
				class: "bg-sr-toggle-inactive",
			},
			{
				game: Variants.SR,
				variant: "active",
				class: "bg-sr-toggle-active",
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
	"w-8 min-h-8 duration-400 flex items-center justify-center",
	{
		variants: {
			game: {
				[Variants.BH]: "bg-white border-2 border-bh-knob-border",
				[Variants.YS]: "bg-[#ece5d8] rounded-full",
				[Variants.SR]: "bg-white rounded-full",
				[Variants.NAP]: "rounded-full bg-nap-knob-bg",
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
	const { game, setGame } = useGame();
	let [enabled, setEanbled] = useState<boolean>(startActive);

	return (
		<div
			onClick={() => {
				setEanbled(!enabled);
				onClick(enabled);
			}}
			class={toggleSwitchStyles({
				game: game,
				variant: `${enabled ? "active" : "inactive"}`,
			})}
		>
			<div
				style={{
					transform: `${enabled ? "translateX(155%)" : ""} translateZ(1px)`,
				}}
				class={toggleSwitchKnobStyles({ game: game })}
			>
				{enabled ? <Check size={18} /> : <X size={18} />}
			</div>
		</div>
	);
}
