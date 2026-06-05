import { cva } from "class-variance-authority";
import { ArrowDown, Check } from "lucide-preact";
import { type MutableRef, useEffect, useRef, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { type ComponentSize, Variants } from "../types";

const dropdownStyles = cva(
	"flex flex-row items-center justify-between transition duration-150",
	{
		variants: {
			game: {
				[Variants.BH3]:
					"bg-bh3-dropdown text-bh3-dropdown-text active:bg-bh3-dropdown-active",
				[Variants.HK4E]:
					"rounded-3xl border-2 border-transparent bg-hk4e-dropdown hover:border-white active:border-hk4e-dropdown-border-active active:bg-hk4e-dropdown-active text-hk4e-dropdown-text",
				[Variants.HKRPG]:
					"rounded-full bg-hkrpg-dropdown-bg hover:bg-white border-2 border-transparent hover:border-hkrpg-dropdown-hover-border active:border-white active:bg-hkrpg-dropdown-active-bg",
				[Variants.NAP]:
					"nap-dots rounded-full border-3 border-[#353535] active:animate-nap-pulsate",
			},
			size: {
				xs: "px-0.75 h-2.5 text-xs",
				sm: "px-1.5 h-5 text-sm",
				md: "px-3 h-10 text-base",
				lg: "px-6 h-20 text-lg",
				xl: "px-12 h-40 text-xl",
			},
		},
	},
);

const dropdownList = cva(
	"min-h-auto absolute flex w-full flex-col transition-opacity duration-250 z-100",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-white text-bh3-dropdown-text",
				[Variants.HK4E]:
					"rounded-[1.25rem] bg-[#495366] drop-shadow-md px-1 py-1",
				[Variants.HKRPG]: "bg-hkrpg-list-bg rounded-xs",
				[Variants.NAP]: "bg-[#353535] rounded-2xl",
			},
			size: {
				xs: "mt-2.5",
				sm: "mt-5",
				md: "mt-10",
				lg: "mt-20",
				xl: "mt-40",
			},
		},
	},
);

const dropdownItem = cva(
	"w-full py-1 px-2 flex flex-row bg-transparent justify-between items-center text-center transition-all duration-175",
	{
		variants: {
			game: {
				[Variants.BH3]: "active:bg-bh3-dropdown-item-active",
				[Variants.HK4E]:
					"hover:bg-ys-item-hover active:bg-[#ece5d8] rounded-4xl text-[#ece5d8] active:text-[#495366]",
				[Variants.HKRPG]:
					"py-5 active:bg-hkrpg-item-active-bg hover:bg-hkrpg-item-hover-bg",
				[Variants.NAP]:
					"text-white active:text-black active:animate-nap-pulsate rounded-full text-center",
			},
			size: {
				xs: "h-2",
				sm: "h-4",
				md: "h-8",
				lg: "h-16",
				xl: "h-32",
			},
		},
	},
);

const getInitialValue = (value: number | string, values: string[]): number => {
	if (typeof value === "number") return value;
	const valueIndex = values.indexOf(value);
	return valueIndex !== -1 ? valueIndex : 0;
};

export const Dropdown = ({
	labels,
	values = labels,
	initialValue = 0,
	onChangeAction,
	width = 7.5,
	height = 1.25,
	size = "md",
}: {
	labels: string[];
	values?: string[];
	initialValue: number | string;
	onChangeAction: (label: string) => void;
	width?: number;
	height?: number;
	size?: ComponentSize;
}) => {
	const { game } = useGame();
	const initialValueIndex = getInitialValue(initialValue, values);
	const [open, setOpen] = useState<boolean>(false);
	const [label, setLabel] = useState<string>(labels[initialValueIndex]);
	const [currentIndex, setCurrentIndex] = useState<number>(initialValueIndex);
	const dialog: MutableRef<HTMLDialogElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return;

		const newLabel = labels[index];
		setLabel(newLabel);
		setOpen(false);
		setCurrentIndex(index);
		onChangeAction(values[index]);
	};

	useEffect(() => {
		if (!open) return;

		const handleOutsideClick = (e: MouseEvent) => {
			if (dialog.current && !dialog.current.contains(e.target as Node)) {
				setOpen(false);
			}
		};

		document.addEventListener("mousedown", handleOutsideClick);
		return () => {
			document.removeEventListener("mousedown", handleOutsideClick);
		};
	}, [open]);

	const containerStyle = {
		width: `${width}rem`,
		height: `${height}rem`,
	};

	return (
		<dialog class="relative" ref={dialog} style={containerStyle}>
			<div class="flex h-full flex-col">
				<button
					type="button"
					class={dropdownStyles({ game, size })}
					onClick={(e) => {
						e.stopPropagation();
						setOpen(!open);
					}}
				>
					<h1>{label}</h1>
					<ArrowDown />
				</button>
				<div
					class={dropdownList({ game, size })}
					style={
						open ? "opacity: 100" : "opacity: 0; pointer-events: none; scale: 0"
					}
				>
					{labels.map((listLabel, index) => (
						<button
							type="button"
							key={index}
							onClick={() => onChange(index)}
							class={dropdownItem({ game, size })}
						>
							<p class="pt-0.5">{listLabel}</p>
							{index === currentIndex ? <Check /> : null}
						</button>
					))}
				</div>
			</div>
		</dialog>
	);
};

export default Dropdown;
