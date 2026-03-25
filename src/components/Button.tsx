import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";

const buttonStyles = cva(
	"transition-all px-5 py-2 duration-175 flex flex-row gap-3 justify-center items-center transiton-all min-h-15 text-[1.25em]",
	{
		variants: {
			game: {
				[Variants.BH3]: "border-2",
				[Variants.HK4E]: "rounded-full drop-shadow-md",
				[Variants.HKRPG]:
					"rounded-full py-3 outline-2 hover:outline-sr-btn-outline active:outline-sr-btn-outline drop-shadow-sm",
				[Variants.NAP]:
					"transition-colors duration-200 flex flex-row justify-center items-center border-3 rounded-full border-nap-btn-border active:border-transparent active:text-black active:animate-nap-pulsate",
			},
			variant: {
				// These are needed so the primary/secondary variants below register properly
				primary: "",
				secondary: "",
			},
		},
		compoundVariants: [
			{
				game: Variants.BH3,
				variant: "primary",
				class:
					"drop-shadow-lg bh-button-primary-dots drop-shadow-bh-btn-primary-shadow border-bh-btn-border",
			},
			{
				game: Variants.BH3,
				variant: "secondary",
				class:
					"border-bh-btn-secondary-border bh-button-secondary-dots bg-gray-800",
			},
			{
				game: Variants.HK4E,
				variant: "primary",
				class:
					"bg-ys-btn-primary text-ys-btn-secondary border-2 border-transparent hover:border-white active:border-ys-btn-primary-border-active active:bg-ys-btn-primary-active active:text-white",
			},
			{
				game: Variants.HK4E,
				variant: "secondary",
				class:
					"bg-ys-btn-secondary text-ys-btn-primary border-2 border-transparent hover:border-btn-secondary-border-hover active:bg-ys-btn-secondary-active active:text-ys-btn-secondary-active-text active:border-ys-btn-secondary-border-active",
			},
			{
				game: Variants.HKRPG,
				variant: "primary",
				class:
					"bg-sr-btn-primary outline-sr-btn-primary-outline hover:bg-sr-btn-primary-hover active:bg-sr-btn-primary-active",
			},
			{
				game: Variants.HKRPG,
				variant: "secondary",
				class:
					"bg-sr-btn-secondary outline-transparent hover:bg-white active:bg-sr-btn-secondary-active",
			},
			{
				game: Variants.NAP,
				variant: "primary",
				class: "nap-dots",
			},
			{
				game: Variants.NAP,
				variant: "secondary",
				class: "bg-transparent",
			},
		],
	},
);

export default function Button({
	onClick,
	children,
	overrideMinWidth = false,
	intent,
}: {
	onClick: () => void;
	children: any;
	overrideMinWidth?: boolean;
	intent: "primary" | "secondary" | null | undefined;
}) {
	const { game } = useGame();
	return (
		<button
			onClick={onClick}
			class={`${buttonStyles({ game: game, variant: intent })} ${!overrideMinWidth ? "min-w-60" : ""}`}
		>
			{children}
		</button>
	);
}
