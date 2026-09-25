import unittest

from pager import paginate


class Paginate(unittest.TestCase):
    def test_first_page(self):
        self.assertEqual(paginate(list(range(10)), 1, 3), ([0, 1, 2], 4))

    def test_last_page_is_partial(self):
        self.assertEqual(paginate(list(range(10)), 4, 3), ([9], 4))


if __name__ == "__main__":
    unittest.main()
