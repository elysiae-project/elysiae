function Diamond() {
	return (
		<div
			className="bg-[#f1f2f3] min-w-5 h-5"
			style={{
				clipPath: "polygon(0px 50%, 10px 0%, 20px 50%, 10px 100%, 0px 50%)",
			}}
		></div>
	);
}

export default function YsProgressbar({ progress }: { progress: number }) {
	return (
		<div className="flex flex-row gap-0 items-center h-10 w-full max-w-md">
			<Diamond />
			<div
				className="bg-[#242424] h-5 flex-1"
				style={{
					clipPath:
						"polygon(0% 50%, 10px 0%, calc(100% - 10px) 0%, 100% 50%, calc(100% - 10px) 100%, 10px 100%, 0% 50%)",
				}}
			>
				<div
					style={{ width: `${progress}%` }}
					className="bg-[#f1f2f3] h-full transition-all duration-300"
				></div>
			</div>
			<Diamond />
		</div>
	);
}
