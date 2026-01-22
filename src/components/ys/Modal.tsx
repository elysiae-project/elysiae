import { Minimize2 } from "lucide-preact";

export default function YsModal({
	children,
	title,
}: {
	children: any;
	title: string;
}) {
	return (
		<dialog class="w-auto min-w-45 h-auto min-h-30 overflow-y-scroll bg-[#3b4354] rounded-lg px-5 py-3">
			<div class="flex flex-row justify-between items-center">
				<h2 class="text-[#d2bc8d]">{title}</h2>
				<Minimize2 color="#d2bc8d" />
			</div>
			<hr class="bg-[#4b5366]"></hr>
			<div>{children}</div>
		</dialog>
	);
}
