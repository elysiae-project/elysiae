import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import Button from "./components/Button.tsx";
import { Download, Trash } from "lucide-preact";
import ToggleSwitch from "./components/ToggleSwitch.tsx";
import YsDropdown from "./components/ys/Dropdown.tsx";
import YsProgressbar from "./components/ys/Progressbar.tsx";

const theme = cva("h-full w-full px-3 py-4", {
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
		<div class="flex h-screen w-screen flex-col gap-0 text-white">
			<Titlebar />
			<div class={theme({ intent: useGame() })}>
				<div class="flex h-full w-full flex-col items-center justify-center text-center gap-2">
					<h1 class="text-8xl">It's Taurin'</h1>
					<h2 class="text-6xl">Time</h2>
					<Button onClick={() => {}} intent="primary">
						<Download /> Download
					</Button>
					<Button onClick={() => {}} intent="secondary">
						<Trash />
					</Button>
					<ToggleSwitch
						startActive={false}
						onClick={(e) => {
							console.log(`Clicked! Value: ${e}`);
						}}
					></ToggleSwitch>
					<YsDropdown
						labels={["Light", "Dark", "System"]}
						initialIndex={0}
						onChangeAction={(e) => {
							console.log(`Selected Index: ${e}`);
						}}
					></YsDropdown>
					<YsProgressbar progress={12.55} />
				</div>
			</div>
		</div>
	);
}
