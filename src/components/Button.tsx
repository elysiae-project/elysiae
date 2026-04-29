import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";

const buttonStyles = cva(
	"transition-all duration-175 px-5 py-2 flex flex-row gap-3 justify-center items-center transiton-all text-[1.25em]",
	{
		variants: {
			game: {
				[Variants.BH3]: "border-2",
				[Variants.HK4E]: "rounded-full drop-shadow-md",
				[Variants.HKRPG]:
					"rounded-full py-3 outline-2 hover:outline-hkrpg-btn-outline active:outline-hkrpg-btn-outline drop-shadow-sm",
				[Variants.NAP]:
					"transition-colors duration-200 flex flex-row justify-center items-center border-3 rounded-full border-nap-btn-border active:border-transparent active:text-black active:animate-nap-pulsate",
			},
			variant: {
				// These are needed so the primary/secondary variants below register properly
				primary: null,
				secondary: null,
			},
			disabled: {
				false: null,
				true: "opacity-50 pointer-events-none",
			},
		},
		compoundVariants: [
			{
				game: Variants.BH3,
				variant: "primary",
				class:
					"drop-shadow-lg bh3-button-primary-dots drop-shadow-bh3-btn-primary-shadow border-bh-btn-border",
			},
			{
				game: Variants.BH3,
				variant: "secondary",
				class:
					"border-bh3-btn-secondary-border bh3-button-secondary-dots bg-gray-800",
			},
			{
				game: Variants.HK4E,
				variant: "primary",
				class:
					"bg-hk4e-btn-primary text-hk4e-btn-secondary border-2 border-transparent hover:border-white active:border-hk4e-btn-primary-border-active active:bg-hk4e-btn-primary-active active:text-white",
			},
			{
				game: Variants.HK4E,
				variant: "secondary",
				class:
					"bg-hk4e-btn-secondary text-hk4e-btn-primary border-2 border-transparent hover:border-btn-secondary-border-hover active:bg-hk4e-btn-secondary-active active:text-hk4e-btn-secondary-active-text active:border-hk4e-btn-secondary-border-active",
			},
			{
				game: Variants.HKRPG,
				variant: "primary",
				class:
					"bg-hkrpg-btn-primary outline-hkrpg-btn-primary-outline hover:bg-hkrpg-btn-primary-hover active:bg-hkrpg-btn-primary-active",
			},
			{
				game: Variants.HKRPG,
				variant: "secondary",
				class:
					"bg-hkrpg-btn-secondary outline-transparent hover:bg-white active:bg-hkrpg-btn-secondary-active",
			},
			{
				game: Variants.NAP,
				variant: "primary",
				class: "nap-dots-titlebar-btn",
			},
			{
				game: Variants.NAP,
				variant: "secondary",
				class: "nap-dots-titlebar-btn",
			},
		],
	},
);

export default function Button({
	onClick,
	children,
	intent: variant,
	disabled = false,
	width = 220,
	height = 65,
}: {
	onClick: () => void;
	children: any;
	intent: "primary" | "secondary";
	disabled?: boolean;
	width?: number;
	height?: number;
}) {
	const { game } = useGame();
	return (
		<button
			disabled={disabled}
			onClick={onClick}
			class={`${buttonStyles({ game: game, variant: variant, disabled: disabled })}`}
			style={{
				minWidth: `${width}px`,
				minHeight: `${height}px`,
			}}>
			{children}
		</button>
	);
}
