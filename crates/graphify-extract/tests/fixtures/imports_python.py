# Variety of Python import forms to exercise import_handlers.
import os
import sys as system
import json.encoder
from collections import OrderedDict, defaultdict
from typing import List, Dict as D
from . import sibling
from .submodule import helper
from ..parent import parent_helper
from .nested.deep import deep_thing

import os.path


def use_imports():
    return os.path.join(sys.argv[0], "x")


class Container:
    def __init__(self):
        self.cache = defaultdict(list)
