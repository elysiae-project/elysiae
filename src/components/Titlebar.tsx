import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";
import { closeApp } from "../util/AppFunctions.ts";
import MenuClose from "./MenuClose.tsx";

const titlebarStyles = cva(
	"h-15 min-w-full p-1 transition-all duration-175 flex flex-row justify-between items-center px-5 py-1.5",
	{
		variants: {
			game: {
				// Declare fonts because the titlebar is declared outside the actual app content. Can be easily fixed if I wasn't lazy
				[Variants.BH]: "bg-bh-titlebar font-bh-sr",
				[Variants.YS]: "bg-ys-titlebar font-ys",
				[Variants.SR]: "bg-sr-titlebar titlebar-sr-noise font-bh-sr",
				[Variants.NAP]:
					"bg-nap-titlebar font-nap border-t-2 border-r-2 border-l-2 border-nap-border rounded-tl-xl",
			},
		},
	},
);

export default function Titlebar() {
	const { game, setGame } = useGame();
	return (
		<div
			style={{ zIndex: 1001, color: "white" }}
			data-tauri-drag-region
			class={titlebarStyles({ game: game })}
		>
			<h3 class="text-center text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
			<MenuClose clickAction={closeApp} />
		</div>
	);
}
