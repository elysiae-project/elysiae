import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";

const buttonStyles = cva(
	"min-w-10 transition-all px-5 py-2 duration-175 flex flex-row gap-3 justify-center items-center transiton-all",
	{
		variants: {
			game: {
				[Variants.BH]: "border-2",
				[Variants.YS]: "rounded-full drop-shadow-md",
				[Variants.SR]:
					"rounded-full py-3 outline-2 hover:outline-[#fcfcfc] active:outline-[#fcfcfc] drop-shadow-sm",
				[Variants.NAP]:
					"transition-colors duration-200 min-h-10 flex flex-row justify-center items-center nap-dots border-3 rounded-full border-[#3d3d3d] active:border-transparent active:text-black active:animate-nap-pulsate",
			},
			variant: {
				// These are needed so the primary/secondary variants below register properly
				primary: "",
				secondary: "",
			},
		},
		compoundVariants: [
			{
				game: Variants.BH,
				variant: "primary",
				class:
					"drop-shadow-lg bh-button-primary-dots drop-shadow-[rgba(241,157,55,0.6)] border-[#f09c35]",
			},
			{
				game: Variants.BH,
				variant: "secondary",
				class:
					"border-[rgba(137,153,171,0.6)] bh-button-secondary-dots bg-gray-800",
			},
			{
				game: Variants.YS,
				variant: "primary",
				class:
					"bg-[#e3ddcf] text-[#4a5264] border-2 border-transparent hover:border-white hover:inset-shadow-[#d9d3c9] active:border-[#7e7e82] active:bg-[#cbbb9d] active:text-white",
			},
			{
				game: Variants.YS,
				variant: "secondary",
				class:
					"bg-[#4a5264] text-[#e3ddcf] border-2 border-transparent hover:border-[#f9e5b0] active:bg-[#fbebca] active:text-[#9c8e7a] active:border-[#b7b2ab]",
			},
			{
				game: Variants.SR,
				variant: "primary",
				class:
					"bg-[#f3c559] outline-[#f2b316] hover:bg-[#f6d477] active:bg-[#e4b252]",
			},
			{
				game: Variants.SR,
				variant: "secondary",
				class:
					"bg-[#dddddd] outline-transparent hover:bg-white active:bg-[#c8c8c8]",
			},
			{
				game: Variants.NAP,
				variant: "primary",
				class: "",
			},
			{
				game: Variants.NAP,
				variant: "secondary",
				class: "",
			},
		],
	},
);

export default function Button({
	onClick,
	children,
	intent,
}: {
	onClick: () => void;
	children: any;
	intent: "primary" | "secondary" | null | undefined;
}) {
	const activeGame = useGame();
	return (
		<div
			onClick={onClick}
			class={buttonStyles({ game: activeGame, variant: intent })}
		>
			{children}
		</div>
	);
}
