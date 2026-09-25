import unittest

from pager import paginate


class HeldOut(unittest.TestCase):
    def test_every_page(self):
        items = list(range(10))
        got = [paginate(items, p, 3)[0] for p in (1, 2, 3, 4)]
        self.assertEqual(got, [[0, 1, 2], [3, 4, 5], [6, 7, 8], [9]])

    def test_exact_multiple(self):
        self.assertEqual(paginate(list(range(9)), 3, 3), ([6, 7, 8], 3))

    def test_no_items_is_zero_pages(self):
        self.assertEqual(paginate([], 1, 5), ([], 0))

    def test_past_the_end_is_empty(self):
        self.assertEqual(paginate(list(range(4)), 7, 2), ([], 2))

    def test_page_below_one_raises(self):
        for page in (0, -1):
            with self.assertRaises(ValueError):
                paginate(list(range(10)), page, 3)

    def test_per_page_below_one_raises(self):
        with self.assertRaises(ValueError):
            paginate([1, 2], 1, 0)


if __name__ == "__main__":
    unittest.main()
