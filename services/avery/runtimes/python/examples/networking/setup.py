from setuptools import setup

setup(
    name="networking",
    version="0.1.0",
    author="GBK Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Example showcasing the firm networking capabilities in Python",
    py_modules=["networking"],
    entry_points={"console_scripts": ["networking=networking:main"]},
)
