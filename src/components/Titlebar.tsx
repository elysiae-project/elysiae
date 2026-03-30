import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";
import { closeApp } from "../util/AppFunctions.ts";
import MenuClose from "./MenuClose.tsx";

const titlebarStyles = cva("h-15 min-w-full p-1 transition-all duration-175", {
	variants: {
		game: {
			// Declare fonts because the titlebar is declared outside the actual app content. Can be easily fixed if I wasn't lazy
			[Variants.BH3]: "bg-bh-titlebar font-bh-sr rounded-t-xl",
			[Variants.HK4E]: "bg-ys-titlebar font-ys",
			[Variants.HKRPG]: "bg-sr-titlebar titlebar-sr-noise font-bh-sr",
			[Variants.NAP]:
				"bg-nap-titlebar nap-dots-titlebar font-nap rounded-tl-xl",
		},
	},
});

export default function Titlebar() {
	const { game } = useGame();
	return (
		<div class={titlebarStyles({ game: game })}>
			<div
				style={{ zIndex: 1001, color: "white" }}
				class="flex flex-row justify-between items-center px-5 py-1.5"
				data-tauri-drag-region
			>
				<h3 class="text-center text-[1.25rem]" data-tauri-drag-region>
					Elysiae
				</h3>
				<MenuClose clickAction={closeApp} />
			</div>
		</div>
	);
}
