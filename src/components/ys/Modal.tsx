import { X } from "lucide-preact";
import { MutableRef, useEffect, useRef, useState } from "preact/hooks";

export default function YsModal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	const dialog: MutableRef<null | HTMLDialogElement> = useRef(null);
	let [isOpen, setIsOpen] = useState(open);
	useEffect(() => {
		if (!dialog.current) return;
		if (isOpen) {
			if (!dialog.current.open) dialog.current.showModal();
		} else {
			if (dialog.current.open) dialog.current.close();
		}
	}, [isOpen]);

	useEffect(() => {
		console.log("Guess what? It's working.....");
		setIsOpen(open);
	}, [open]);

	if (!isOpen) return null;
	return (
		<div
			class="w-full h-full flex justify-center items-center fixed inset-0"
			style={{
				backdropFilter: "blur(7px)",
				backgroundColor: "oklch(0.1574 0 82 / 60%)",
				zIndex: 1000,
				pointerEvents: "none",
			}}
			onClick={() => setIsOpen(false)}
		>
			<div
				class="w-auto min-w-125 h-auto min-h-75 overflow-y-scroll bg-[#3b4354] rounded-md px-5 py-3 text-white"
				style={{ pointerEvents: "auto" }}
				onClick={(e) => e.stopPropagation()}
			>
				<div class="flex flex-row justify-between items-center w-full mb-1">
					<h2 class="text-[#d2bc8d]">{title}</h2>
					<div
						class=" cursor-pointer border-5 border-[#888d8e] bg-[#ece5d8] hover:border-transparent hover:drop-shadow-xs hover:drop-shadow-[#fdfdfeAA] flex items-center justify-center
						active:bg-[#9a947f] active:border-transparent p-2 rounded-full h-10 w-10"
						onClick={() => setIsOpen(false)}
					>
						<X size={24} color="#4b5366" />
					</div>
				</div>
				<hr style={{ color: "#69758f" }} class="mb-2.5 mx-[-21]"></hr>
				{children}
			</div>
		</div>
	);
}
