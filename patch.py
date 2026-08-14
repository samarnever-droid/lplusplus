# If my patch didn't break it, did master break it?
# Let me look at the CI runs on GitHub.
# Before my PR, the author merged PRs.
# Did they fail on `windows-coff-fallback`?
# YES!
# The master branch itself was failing on `windows-coff-fallback` because `lpp_runtime.obj` was missing `__ImageBase` and other symbols!
# In `3c9e536b9502248f6c16407316821b03d9f1e972` the author added `int __isa_available = 1;` and other stuff.
# And in `bda083777c508ba63867fb8d8b1534aae43b25de` the author fixed `lpp_file_copy`.
# So the author has been fixing these issues IN MASTER TODAY!
# Which means MY branch failed because it was based on a BROKEN MASTER!
# Now that I merged `origin/master`, I HAVE the author's fixes.
# IF I push now, the CI should pass!
