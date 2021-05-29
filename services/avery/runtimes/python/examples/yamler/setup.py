""" Setup the yamler """
from setuptools import setup

setup(
    name="python-package-bundling-example",
    version="0.1.0",
    author="GBK Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Tests python wheel dependency bundling",
    py_modules=["yamler"],
    entry_points={"console_scripts": ["yamler=yamler:main"]},
)
