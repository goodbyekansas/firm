from setuptools import setup, find_packages

setup(
    name="firm-protocols",
    version="0.1.0",
    url="https://github.com/goodbyekansas/firm",
    author="GBK Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Python type definitions for Firm protocols",
    packages=find_packages(),
    package_data={"firm_protocols": ["**/*.pyi", "**/py.typed", "py.typed", "*.pyi"]},
    include_package_data=True,
    zip_safe=False,
)
