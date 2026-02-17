import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { MutableRef, useEffect, useRef, useState } from "preact/hooks";
import { ArrowDown, Check } from "lucide-preact";

const dropdownStyles = cva(
	"flex flex-row h-10 px-3 items-center justify-between",
	{
		variants: {
			game: {
				[Variants.BH]: "bg-[#f5f5f5] text-[#45424d] active:bg-[#dcdcdc]",
				[Variants.YS]:
					"rounded-3xl border-2 border-transparent bg-[#dcd5c9] hover:border-white active:border-[#444140] active:bg-[#edd4b2] text-[#3d4557]",
				[Variants.SR]:
					"rounded-full bg-[#e6e5e3] hover:bg-[#ffffff] border-2 border-transparent hover:border[#eeeeee] active:border-white active:bg-[#cfcecd]",
				[Variants.NAP]:
					"nap-dots rounded-full border-3 border-[#353535] active:animate-nap-pulsate",
			},
		},
	},
);

const dropdownListStyles = cva(
	"min-h-auto mt-10.5 absolute flex w-full flex-col transition-opacity duration-150",
	{
		variants: {
			game: {
				[Variants.BH]: "bg-white text-[#45424d]",
				[Variants.YS]:
					"rounded-[1.25rem] bg-[#495366] drop-shadow-md px-1 py-1 transition-opacity duration-150",
				[Variants.SR]: "bg-[#ededed] mt-12 rounded-xs",
				[Variants.NAP]: "bg-[#353535] rounded-2xl",
			},
		},
	},
);

const dropdownItemStyles = cva(
	"w-full h-8 py-1 px-2 flex flex-row bg-transparent justify-between items-center text-center transition-all duration-175",
	{
		variants: {
			game: {
				[Variants.BH]: "active:bg-[#bfbfbf]",
				[Variants.YS]:
					"hover:bg-[#606979] active:bg-[#ece5d8] rounded-4xl text-[#ece5d8] active:text-[#495366]",
				[Variants.SR]: "py-5 active:bg-[#bdbdbd] hover:bg-[#f0f0f0]",
				[Variants.NAP]:
					"text-white active:text-black active:animate-nap-pulsate rounded-full text-center",
			},
		},
	},
);

export default function Dropdown({
	labels,
	initialIndex = 0,
	onChangeAction,
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (index: number) => void;
}) {
	const activeGame = useGame();

	let [open, setOpen] = useState<boolean>(false);
	let [label, setLabel] = useState<string>(labels[initialIndex]);
	let [currentIndex, setCurrentIndex] = useState<number>(initialIndex);

	const dropdownDiv: MutableRef<HTMLDivElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return;
		setLabel(labels[index]);
		setOpen(false);
		setCurrentIndex(index);
		onChangeAction(index);
	};

	useEffect(() => {
		const onClick = (e: MouseEvent) => {
			setOpen(false);
		};
		if (open) {
			document.addEventListener("click", onClick);
		}
		return () => {
			document.removeEventListener("click", onClick);
		};
	}, [open]);

	return (
		<div class="relative w-60" ref={dropdownDiv} style={{ zIndex: 500 }}>
			<div class="overflow-y-auto flex h-full flex-col">
				<div
					class={dropdownStyles({ game: activeGame })}
					onClick={(e) => {
						e.stopPropagation();
						setOpen(!open);
					}}
				>
					<h1>{label}</h1>
					<ArrowDown />
				</div>
				<div
					class={dropdownListStyles({ game: activeGame })}
					style={open ? "opacity: 100" : "opacity: 0;"}
				>
					{labels.map((listLabel, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class={dropdownItemStyles({ game: activeGame })}
							>
								<p class="pt-0.5">{listLabel}</p>
								{index === currentIndex ? <Check /> : null}
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}
