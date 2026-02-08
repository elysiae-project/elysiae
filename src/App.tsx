import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";

const theme = cva("h-full w-full px-3 py-4 flex-1", {
	variants: {
		intent: {
			[Variants.BH]: "bg-bh-bg font-bh-sr rounded-b-xl text-white",
			[Variants.YS]: "bg-ys-bg font-ys text-black",
			[Variants.SR]:
				"bg-sr-bg font-bh-sr rounded-b-xs border border-[#393939] text-black",
			[Variants.NAP]:
				"bg-nap-bg font-nap rounded-br-xl border-b-2 border-r-2 border-l-2 border-nap-border text-white nap-dots",
		},
	},
});

export default function App() {
	return (
		<div class="flex h-screen w-screen flex-col gap-0">
			<Titlebar />
			<div class={theme({ intent: useGame() })}></div>
		</div>
	);
}
