""" make firm-api a python package """
from setuptools import setup

setup(
    name="firm-api",
    version="1.0.0",
    author="GBK Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Example showcasing the Firm API in Python",
    py_modules=["firm_api"],
    entry_points={
        "console_scripts": [
            "firm-api=firm_api:main",
            "firm-api-error=firm_api:main_with_error",
        ],
    },
)
