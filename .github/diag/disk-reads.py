# Throwaway diagnosis for with-hide/hide-private issue 354, never merged.
# Reads tracerpt's CSV of an NT Kernel Logger trace and sums the disk reads by
# file (DiskIo Read's FileObject, named by the FileIo events) and by disk.
import collections, csv, sys

fields = {}
names = {}
reads = collections.Counter()
counts = collections.Counter()
by_disk = collections.Counter()
faults = collections.Counter()
with open(sys.argv[1], encoding="utf-8", errors="replace", newline="") as source:
    for row in csv.reader(source, skipinitialspace=True):
        if len(row) < 3:
            continue
        event, kind = row[0].strip(), row[1].strip()
        if row[2].strip() == "TID":
            fields[(event, kind)] = [c.strip() for c in row]
            continue
        header = fields.get((event, kind))
        if not header:
            continue
        record = dict(zip(header, (c.strip() for c in row)))
        file_object = record.get("FileObject") or record.get("FileKey")
        name = record.get("FileName") or record.get("OpenPath")
        if event == "FileIo" and file_object and name:
            names[file_object] = name
        if event == "DiskIo" and kind == "Read":
            size = int(record.get("TransferSize", "0") or 0, 0)
            disk = record.get("DiskNumber", "?")
            by_disk[disk] += size
            key = (disk, file_object)
            reads[key] += size
            counts[key] += 1
        if event == "PageFault" and kind == "HardFault":
            fo = record.get("FileObject")
            faults[fo] += int(record.get("ByteCount", "0") or 0, 0)
print("disk reads by disk (MB):", {d: round(b / 2**20) for d, b in by_disk.items()})
print("top files read from disk:")
for (disk, fo), size in reads.most_common(40):
    print(f"  disk {disk} {size / 2**20:8.1f} MB {counts[(disk, fo)]:6} reads  {names.get(fo, fo)}")
if faults:
    print("top files by hard faults:")
    for fo, size in faults.most_common(25):
        print(f"  {size / 2**20:8.1f} MB  {names.get(fo, fo)}")
