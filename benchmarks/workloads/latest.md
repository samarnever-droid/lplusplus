# Native workload-shape matrix

| Workload | Backend | Emit ms | Link ms | Run ms | Status |
|---|---|---:|---:|---:|---|
| arithmetic | c | 2.103 | 144.407 | 1.357 | PASS |
| arithmetic | aot | 2.686 | 198.452 | 7.478 | PASS |
| branches | c | 1.852 | 55.456 | 13.361 | PASS |
| branches | aot | 2.771 | 197.890 | 52.350 | PASS |
| calls | c | 2.122 | 52.034 | 1.285 | PASS |
| calls | aot | 2.667 | 203.226 | 17.015 | PASS |
| struct_list | c | 2.023 | 68.425 | 2.072 | PASS |
| struct_list | aot | 2.915 | 200.282 | 2.066 | PASS |
| list_labyrinth | c | 124.418 | 9408.980 | 1.433 | PASS |
| list_labyrinth | aot | 431.972 | 215.712 | 1.447 | PASS |
