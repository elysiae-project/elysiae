import { useState } from "preact/hooks";
import { Check, X } from "lucide-preact";

export default function YsToggleSwitch({
	onClick,
	startActive = false,
}: {
	onClick: (enabled: boolean) => void;
	startActive: boolean;
}) {
	let [enabled, setEnabled] = useState<boolean>(startActive);

	return (
		<div
			class={
				"w-25 min-h-8 p-1.5 rounded-full border-3 border-white transition-colors duration-200 delay-0 " +
				(enabled ? "bg-[#dccba9]" : "bg-[#353d4f]")
			}
			onClick={() => {
				setEnabled(!enabled);
				onClick(enabled);
			}}
		>
			<div
				class="rounded-full w-8 min-h-8 inner-switch p-1 bg-[#ece5d8] transition-transform duration-400 delay-0"
				style={{ translate: enabled ? "155%" : "0%" }}
			>
				{enabled ? (
					<Check style="color: #6d7887" />
				) : (
					<X style="color: #ccb68a" />
				)}
			</div>
		</div>
	);
}
