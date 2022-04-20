""" Package setup for the component sexparser """
from setuptools import find_packages, setup

setup(
    name="turbo-isl-compiler",
    version="0.1.0",
    url="https://github.com/goodbyekansas/firm",
    author="Goodbye Kansas Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Generates rust code for the firm abi",
    packages=find_packages(),
    entry_points={"console_scripts": ["turbo=turbo_islc.main:main"]},
    package_data={"turbo_islc": ["**/templates/**/*"]},
)
