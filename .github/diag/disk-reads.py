# Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
# Reads tracerpt's CSV of an NT Kernel Logger trace (user data is positional,
# from column 19) and sums C: disk reads and hard faults by file and process.
import collections, csv, sys

def number(text):
    try:
        return int(text, 0)
    except ValueError:
        try:
            return int(text)
        except ValueError:
            return 0

names = {}
images = {}
disk_reads = collections.Counter()
disk_counts = collections.Counter()
by_disk = collections.Counter()
fault_bytes = collections.Counter()
fault_counts = collections.Counter()
fault_by_process = collections.Counter()
kinds = collections.Counter()
with open(sys.argv[1], encoding="utf-8", errors="replace", newline="") as source:
    for row in csv.reader(source, skipinitialspace=True):
        if len(row) < 20:
            continue
        event, kind = row[0].strip(), row[1].strip()
        data = [c.strip() for c in row[19:]]
        pid = number(row[9].strip())
        kinds[(event, kind)] += 1
        if event == "FileIo" and kind in ("FileCreate", "FileRundown", "Name", "FileDelete") and len(data) >= 2:
            names[data[0]] = data[1]
        elif event == "Process" and kind in ("Start", "DCStart") and len(data) >= 2:
            image = next((c for c in data if c.lower().endswith(".exe")), None)
            if image:
                images[number(data[1])] = image
        elif event == "DiskIo" and kind == "Read" and len(data) >= 6:
            disk, size, file_object = data[0], number(data[2]), data[5]
            by_disk[disk] += size
            disk_reads[(disk, file_object)] += size
            disk_counts[(disk, file_object)] += 1
        elif event == "PageFault" and kind == "HardFault" and len(data) >= 6:
            file_object, size = data[3], number(data[5])
            fault_bytes[file_object] += size
            fault_counts[file_object] += 1
            fault_by_process[pid] += size
print("events:", dict(kinds.most_common(14)))
print("disk reads by disk (MB):", {d: round(b / 2**20) for d, b in by_disk.items()})
print("top files read from disk:")
for (disk, fo), size in disk_reads.most_common(40):
    print(f"  disk {disk} {size / 2**20:8.1f} MB {disk_counts[(disk, fo)]:6} reads  {names.get(fo, fo)}")
print("top files by hard faults:")
for fo, size in fault_bytes.most_common(30):
    print(f"  {size / 2**20:8.1f} MB {fault_counts[fo]:6} faults  {names.get(fo, fo)}")
print("hard faults by process:")
for pid, size in fault_by_process.most_common(15):
    print(f"  {size / 2**20:8.1f} MB  pid {pid} {images.get(pid, '?')}")
