import { Shrink, Minus } from "lucide-preact";
import YsButton from "./Button";

export default function YsTitlebarButtons({
	close,
	minimize,
}: {
	close: () => void;
	minimize: () => void;
}) {
	const content = [
		{
			icon: Shrink,
			action: close,
		},
		{
			icon: Minus,
			action: minimize,
		},
	] as const;

	return (
		<div class="flex flex-row-reverse gap-2 items-center justify-center overflow-visible">
			{content.map((item, key) => {
				const Icon = item.icon;
				return (
					<div
						class="border-5 border-[#888d8e] bg-[#ece5d8] hover:border-transparent hover:drop-shadow-xs hover:drop-shadow-[#fdfdfeAA] flex items-center justify-center
						active:bg-[#9a947f] active:border-transparent p-2 rounded-full h-10 w-10"
						onClick={item.action}
					>
						<Icon key={key} color="#4b5366" />
					</div>
				);
			})}
		</div>
	);
}
