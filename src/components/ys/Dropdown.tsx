import { Check } from "lucide-preact";
import { MutableRef, useEffect, useRef, useState } from "preact/hooks";

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
	let [currentIndex, setCurrentIndex] = useState<number>(initialIndex);

	const dropdownDiv: MutableRef<HTMLDivElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return; // Permanent temp fix lol
		setLabel(labels[index]);
		setOpen(false);
		setCurrentIndex(index);
		onChangeAction(index);
	};

	return (
		<div class="relative w-60" ref={dropdownDiv}>
			<div class="overflow-y-auto-y flex h-full flex-col">
				<div
					class="flex h-10 items-center justify-center rounded-3xl border-2 border-transparent bg-[#dcd5c9] hover:border-white active:border-[#444140] active:bg-[#edd4b2]"
					onClick={() => setOpen(!open)}
				>
					<h1 class="text-lg text-[#3d4557]">{label}</h1>
				</div>
				<div
					class="min-h-auto mt-10.5 absolute flex w-full flex-col rounded-xl bg-[#495366] px-3 py-2 transition-opacity duration-100"
					style={open ? "opacity: 100" : "opacity: 0;"}
				>
					{labels.map((label, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class={
									"bg-transparent hover:bg-[#606979] active:bg-[#ece5d8] rounded-2xl text-[#ece5d8] active:text-[#495366] w-full py-1 px-2 "
								}
							>
								<p class="flex flex-row justify-between text-center">
									{label} {index === currentIndex ? <Check /> : ""}
								</p>
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}
