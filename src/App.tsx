import "./App.css";
import { useGame } from "./hooks/useGame.ts";
import Titlebar from "./components/Titlebar.tsx";
import { cva } from "class-variance-authority";
import { Variants } from "./types";
import ToggleSwitch from "./components/ToggleSwitch.tsx";
import Button from "./components/Button.tsx";
import Dropdown from "./components/Dropdown.tsx";
import Progressbar from "./components/Progressbar.tsx";

const theme = cva("h-full w-full px-3 py-4 flex flex-col gap-y-2", {
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
			<div class={theme({ intent: useGame() })}>
				<ToggleSwitch startActive={false} onClick={() => {}} />
				<Button intent={"primary"} onClick={() => {}}>
					Primary Button
				</Button>
				<Button intent={"secondary"} onClick={() => {}}>
					Secondary Button
				</Button>
				<Dropdown
					labels={["On", "Off", "Default"]}
					initialIndex={0}
					onChangeAction={() => {}}
				/>
				<Progressbar progress={35} />
			</div>
		</div>
	);
}
