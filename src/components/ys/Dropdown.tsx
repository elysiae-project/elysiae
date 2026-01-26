import { useState } from "preact/hooks";

export default function YsDropdown({
	labels,
	initialIndex = 0,
	onChangeAction,
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (index: number) => void;
}) {
	let [open, setOpen] = useState<boolean>(false);
	let [label, setLabel] = useState<string>(labels[initialIndex]);

	const onChange = (index: number) => {
		setLabel(labels[index]);
		onChangeAction(index);
	};

	return (
		<div class="relative w-60">
			<div
				class="flex flex-col h-auto overflow-y-auto bg-[#dcd5c9] rounded-full border-2 border-transparent hover:border-white active:border-[#444140] active:bg-[#edd4b2]"
				onClick={() => setOpen(!open)}
			>
				<h1 class="text-[#3d4557]">{label}</h1>
				<div style={{ scale: 0 }} class="h-auto p-1 rounded-full bg-[#495366]">
					{labels.map((label, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class="absolute bg-transparent hover:bg-[#606979] active:bg[#ece5d8] text-[#ece5d8] active:text-[#495366] w-full"
							>
								<p>{label}</p>
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}
