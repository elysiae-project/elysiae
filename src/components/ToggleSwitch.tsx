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
				[Variants.BH3]: "rounded-none",
				[Variants.HK4E]: "rounded-full border-2 border-white",
				[Variants.HKRPG]: "rounded-full",
				[Variants.NAP]: "rounded-full border-4 border-nap-dot-border",
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
	"w-8 min-h-8 duration-400 flex items-center justify-center",
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

export default function ToggleSwitch({
	onClick,
	startActive = false,
}: {
	onClick: (enabled: boolean) => void;
	startActive?: boolean;
}) {
	const { game } = useGame();
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
