import { getCurrentWindow } from "@tauri-apps/api/window";
import { cva } from "class-variance-authority";
import { AnimatePresence } from "motion/react";
import { useGame } from "../hooks/useGame.ts";
import MenuClose from "./MenuClose.tsx";
import { Variants } from "../types";

const titlebarStyles = cva(
	"h-16 min-w-full p-1 transition-all duration-250 overflow-y-hidden",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-bh3-titlebar font-bh3-hkrpg rounded-t-xl",
				[Variants.HK4E]: "bg-hk4e-titlebar font-hk4e",
				[Variants.HKRPG]:
					"bg-hkrpg-titlebar titlebar-hkrpg-noise font-bh3-hkrpg",
				[Variants.NAP]:
					"bg-nap-titlebar nap-dots-titlebar font-nap rounded-tl-xl",
			},
		},
	},
);

export default function Titlebar() {
	const { game } = useGame();

	return (
		<div class={titlebarStyles({ game: game })}>
			<div
				style={{ zIndex: 1005, color: "white" }}
				class="flex flex-row items-center justify-between px-5 py-1.5"
				data-tauri-drag-region>
				<h1 class="text-center text-[1.35rem]">Elysiae</h1>
				<AnimatePresence mode="wait" initial={false}>
					<MenuClose
						size={42}
						clickAction={() => {
							getCurrentWindow().close();
						}}
					/>
				</AnimatePresence>
			</div>
		</div>
	);
}
