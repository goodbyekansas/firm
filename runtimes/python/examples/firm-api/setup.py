from setuptools import setup

setup(
    name="firm-api",
    version="0.1.0",
    author="GBK Pipeline Team",
    author_email="pipeline@goodbyekansas.com",
    description="Example showcasing the Firm API in Python",
    py_modules=["firm_api"],
    entry_points={"console_scripts": ["firm-api=firm_api:main"]},
)
