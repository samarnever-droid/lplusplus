# Native workload-shape matrix

| Workload | Backend | Emit ms | Link ms | Run ms | Status |
|---|---|---:|---:|---:|---|
| arithmetic | c | 1.937 | 52.975 | 1.346 | PASS |
| arithmetic | aot | 2.431 | 194.844 | 7.364 | PASS |
| branches | c | 1.801 | 51.181 | 13.059 | PASS |
| branches | aot | 2.566 | 194.667 | 51.923 | PASS |
| calls | c | 1.949 | 48.912 | 1.241 | PASS |
| calls | aot | 2.510 | 197.202 | 16.543 | PASS |
| struct_list | c | 1.831 | 63.636 | 1.948 | PASS |
| struct_list | aot | 2.511 | 198.102 | 1.943 | PASS |
| list_labyrinth | c | 118.730 | 9785.889 | 1.336 | PASS |
| list_labyrinth | aot | 462.147 | 212.076 | 1.382 | PASS |
