import unittest

from coverage import summarize


class CoverageChecks(unittest.TestCase):
    def test_rust_counts_unique_lines_not_overlapping_function_totals(self):
        data = "SF:src/core.rs\nDA:5,3\nDA:6,0\nLF:4\nLH:3\nend_of_record"
        lcov, summary, failures = summarize("rust", data, {"src/core.rs"})
        self.assertIn("LF:2\nLH:1\n", lcov)
        self.assertIn("1/2 (50.00%)", summary)
        self.assertTrue(failures)
        for records, files in [(data * 2, {"src/core.rs"}), (data, {"src/core.rs", "src/new.rs"})]:
            with self.assertRaises(ValueError):
                summarize("rust", records, files)

    def test_full_line_coverage_cannot_hide_a_branch_regression(self):
        data = "SF:web/src/view.ts\nDA:7,1\n" + "".join(f"BRDA:7,0,{i},{int(i < 4)}\n" for i in range(5))
        files = {"web/src/view.ts"}
        self.assertFalse(summarize("dashboard", data + "end_of_record", files)[2])
        failed = summarize("dashboard", data.replace("BRDA:7,0,3,1", "BRDA:7,0,3,0") + "end_of_record", files)[2]
        self.assertEqual(failed, ["branches: 3/5 does not meet 80%"])


if __name__ == "__main__":
    unittest.main()
